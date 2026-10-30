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
