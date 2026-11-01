//! Мультимедиа: что играет, пауза, следующий трек, перемотка.
//!
//! Сначала пробуем системные медиа-сессии (SMTC — так делают Spotify, браузеры
//! и «Плеер»), а если плеер их не отдаёт — жмём мультимедийные клавиши.

use capri_core::platform::{MediaCommand, KeyStroke, Track};

mod media_keys {
    pub const PLAY_PAUSE: u16 = 0xB3;
    pub const NEXT: u16 = 0xB0;
    pub const PREV: u16 = 0xB1;
    pub const STOP: u16 = 0xB2;
}

fn press(vk: u16) -> Result<(), String> {
    crate::input::send_keys(&[KeyStroke { vk, extended: true }])
}

#[cfg(windows)]
fn smtc_session() -> Option<windows::Media::Control::GlobalSystemMediaTransportControlsSession> {
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;
    let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
        .ok()?
        .join()
        .ok()?;
    manager.GetCurrentSession().ok()
}

/// Отправить команду плееру.
pub fn media_command(cmd: MediaCommand) -> Result<(), String> {
    #[cfg(windows)]
    {
        if let Some(session) = smtc_session() {
            let done = match cmd {
                MediaCommand::PlayPause => session.TryTogglePlayPauseAsync().map(|op| op.join().unwrap_or(false)),
                MediaCommand::Play => session.TryPlayAsync().map(|op| op.join().unwrap_or(false)),
                MediaCommand::Pause => session.TryPauseAsync().map(|op| op.join().unwrap_or(false)),
                MediaCommand::Next => session.TrySkipNextAsync().map(|op| op.join().unwrap_or(false)),
                MediaCommand::Prev => session.TrySkipPreviousAsync().map(|op| op.join().unwrap_or(false)),
                MediaCommand::Stop => session.TryStopAsync().map(|op| op.join().unwrap_or(false)),
            };
            if matches!(done, Ok(true)) {
                return Ok(());
            }
        }
    }
    match cmd {
        MediaCommand::PlayPause | MediaCommand::Play | MediaCommand::Pause => press(media_keys::PLAY_PAUSE),
        MediaCommand::Next => press(media_keys::NEXT),
        MediaCommand::Prev => press(media_keys::PREV),
        MediaCommand::Stop => press(media_keys::STOP),
    }
}

/// Что сейчас играет.
pub fn now_playing() -> Option<Track> {
    #[cfg(windows)]
    {
        use windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackStatus;
        let session = smtc_session()?;
        let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
        let title = props.Title().map(|s| s.to_string()).unwrap_or_default();
        if title.trim().is_empty() {
            return None;
        }
        let artist = props.Artist().map(|s| s.to_string()).unwrap_or_default();
        let app = session
            .SourceAppUserModelId()
            .map(|s| s.to_string())
            .unwrap_or_default();

        let playing = session
            .GetPlaybackInfo()
            .ok()
            .and_then(|info| info.PlaybackStatus().ok())
            .map(|s| s == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing)
            .unwrap_or(false);

        let (position, duration) = match session.GetTimelineProperties() {
            Ok(tl) => {
                let pos = tl.Position().map(|d| d.Duration as f64 / 10_000_000.0).unwrap_or(0.0);
                let end = tl.EndTime().map(|d| d.Duration as f64 / 10_000_000.0).unwrap_or(0.0);
                (Some(pos), Some(end))
            }
            Err(_) => (None, None),
        };

        return Some(Track {
            title: title.trim().to_string(),
            artist: artist.trim().to_string(),
            app: short_app_name(&app),
            duration,
            position,
            playing,
        });
    }
    #[cfg(not(windows))]
    None
}

/// Короткое имя плеера для виджета: `Spotify`, `Яндекс Браузер`, …
fn short_app_name(app_id: &str) -> String {
    let lower = app_id.to_lowercase();
    for (needle, name) in [
        ("spotify", "Spotify"),
        ("chrome", "Chrome"),
        ("msedge", "Edge"),
        ("firefox", "Firefox"),
        ("yandex", "Яндекс Браузер"),
        ("opera", "Opera"),
        ("vlc", "VLC"),
        ("wmplayer", "Windows Media Player"),
        ("zunemusic", "Музыка"),
        ("telegram", "Telegram"),
    ] {
        if lower.contains(needle) {
            return name.to_string();
        }
    }
    app_id.rsplit(['\\', '.']).next().unwrap_or(app_id).to_string()
}

/// Обложка текущего трека в виде data-URL (`data:image/png;base64,…`).
///
/// Панель живёт в WebView2 и странице запрещено ходить на диск, поэтому
/// картинку отдаём прямо в JS. Кадр маленький (обложка — несколько десятков
/// килобайт), но чтобы не гонять её на каждом тике, ядро кэширует результат:
/// см. `media_cover` в оболочке.
#[cfg(windows)]
pub fn cover_data_url() -> Option<String> {
    use windows::Storage::Streams::DataReader;

    let session = smtc_session()?;
    let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let thumb = props.Thumbnail().ok()?;

    // Открыть поток обложки и прочитать его целиком.
    let stream = thumb.OpenReadAsync().ok()?.join().ok()?;
    let size = stream.Size().ok()? as u32;
    if size == 0 {
        return None;
    }
    // Обложки больше нескольких мегабайт — это уже не обложка.
    if size > 4 * 1024 * 1024 {
        return None;
    }

    let reader = DataReader::CreateDataReader(&stream).ok()?;
    reader.LoadAsync(size).ok()?.join().ok()?;
    let mut buffer = vec![0u8; size as usize];
    reader.ReadBytes(&mut buffer).ok()?;

    Some(format!(
        "data:image/png;base64,{}",
        base64_encode(&buffer)
    ))
}

#[cfg(not(windows))]
pub fn cover_data_url() -> Option<String> {
    None
}

/// Кодирование в base64 без внешних крейтов.
///
/// Обложка — единственное место, где нужен base64, и ради него тянуть
/// зависимость не стали: алгоритм укладывается в двадцать строк.
#[cfg(windows)]
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(TABLE[(triple >> 18) as usize & 0x3f] as char);
        out.push(TABLE[(triple >> 12) as usize & 0x3f] as char);
        // Последние два символа зависят от того, сколько байт осталось.
        out.push(if chunk.len() > 1 {
            TABLE[(triple >> 6) as usize & 0x3f] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[triple as usize & 0x3f] as char
        } else {
            '='
        });
    }
    out
}

/// Перемотать текущий трек/видео на `seconds` от начала.
pub fn seek(seconds: f64) -> Result<(), String> {
    #[cfg(windows)]
    {
        if let Some(session) = smtc_session() {
            let target = (seconds.max(0.0) * 10_000_000.0) as i64;
            if let Ok(op) = session.TryChangePlaybackPositionAsync(target) {
                if op.join().unwrap_or(false) {
                    return Ok(());
                }
            }
        }
    }
    let _ = seconds;
    Err("этот плеер не даёт перематывать".into())
}

/// Есть ли вообще активная медиа-сессия.
pub fn has_session() -> bool {
    #[cfg(windows)]
    {
        return smtc_session().is_some();
    }
    #[cfg(not(windows))]
    false
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc_vectors() {
        // Векторы из RFC 4648 §10: три случая на добивку.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_handles_high_bytes() {
        // Байты со старшим битом не должны терять знак.
        assert_eq!(base64_encode(&[0xff, 0xfe, 0xfd]), "//79");
        assert_eq!(base64_encode(&[0x00, 0x00, 0x00]), "AAAA");
    }

    #[test]
    fn app_names_are_shortened() {
        assert_eq!(short_app_name("Spotify.exe"), "Spotify");
        assert_eq!(
            short_app_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
            "Музыка"
        );
        assert_eq!(short_app_name("chrome.exe"), "Chrome");
        // Незнакомый идентификатор отдаём как есть, а не пустой строкой.
        assert!(!short_app_name("Foo.Bar").is_empty());
    }
}
