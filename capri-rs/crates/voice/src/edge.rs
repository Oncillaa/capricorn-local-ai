//! Синтез речи через Edge TTS (тот же сервис, что у `edge-tts` в Python).
//!
//! Работает по WebSocket: отправляем конфиг и SSML, получаем обратно
//! MP3-куски. Ключей не нужно — сервис открыт для браузера Edge, но
//! с 2024 года требует заголовок `Sec-MS-GEC` (подпись времени).
//!
//! Подпись — это SHA-256 от «числа тиков» и открытого токена. Тики считаются
//! от 1601 года и округляются вниз до пяти минут — так же, как в оригинальном
//! `edge-tts`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use sha2::{Digest, Sha256};
use tungstenite::Message;

/// Открытый токен клиента Edge (не секрет — он в самом браузере).
const TRUSTED_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
/// Версия клиента в заголовке подписи.
const GEC_VERSION: &str = "1-130.0.2849.68";
const OUTPUT_FORMAT: &str = "audio-24khz-48kbitrate-mono-mp3";
/// Секунд между эпохами Windows (1601) и Unix (1970).
const WIN_EPOCH: u64 = 11_644_473_600;

/// Ошибка синтеза — наверх уходит русский текст для лога и для ответа.
pub type EdgeResult<T> = Result<T, String>;

/// Подпись времени для `Sec-MS-GEC`.
///
/// `offset` сдвигает часы — иногда сервер «отстаёт», и подпись с текущим
/// временем не принимается.
fn gec_token_at(unix_secs: u64) -> String {
    // Округляем вниз до пяти минут и переводим в 100-наносекундные тики.
    let rounded = (unix_secs + WIN_EPOCH) / 300 * 300;
    let ticks = rounded * 10_000_000;
    let digest = Sha256::digest(format!("{ticks}{TRUSTED_TOKEN}").as_bytes());
    digest.iter().map(|b| format!("{b:02X}")).collect()
}

fn gec_token() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    gec_token_at(now)
}

fn timestamp() -> String {
    // Формат: «Mon Jan 01 2024 12:00:00 GMT+0000 (Coordinated Universal Time)»
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (now / 86_400) as i64;
    let secs_of_day = now % 86_400;
    let (hour, minute, second) = (secs_of_day / 3600, (secs_of_day % 3600) / 60, secs_of_day % 60);

    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    // 1 января 1970 — четверг.
    let weekday = WEEKDAYS[(days % 7) as usize];
    let (year, month, day) = civil_from_days(days);
    format!(
        "{weekday} {} {} {year} {hour:02}:{minute:02}:{second:02} GMT+0000 (Coordinated Universal Time)",
        MONTHS[(month - 1) as usize],
        day
    )
}

/// Дни от 1970-01-01 → (год, месяц, день). Алгоритм Говарда Хиннанта.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Идентификатор запроса: 32 hex-символа.
fn request_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut state = nanos as u64 ^ 0x5DEE_CE66_D1CE_4E5B;
    let mut out = String::with_capacity(32);
    for _ in 0..4 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        out.push_str(&format!("{:08x}", (state >> 32) as u32));
    }
    out
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Синтезировать текст в MP3 (один кусок аудио на всю фразу).
///
/// `voice` — например `ru-RU-SvetlanaNeural`, `rate` и `pitch` — как в
/// настройках (`+5%`, `+8Hz`).
pub fn synthesize(text: &str, voice: &str, rate: &str, pitch: &str) -> EdgeResult<Vec<u8>> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let gec = gec_token();
    let url = format!(
        "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1?\
         TrustedClientToken={TRUSTED_TOKEN}&Sec-MS-GEC={gec}&Sec-MS-GEC-Version={GEC_VERSION}&\
         ConnectionId={}",
        request_id()
    );
    let request = tungstenite::http::Request::builder()
        .uri(&url)
        .header("Host", "speech.platform.bing.com")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
        .header("Origin", "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0",
        )
        .header("Pragma", "no-cache")
        .header("Cache-Control", "no-cache")
        .body(())
        .map_err(|e| format!("не собрала запрос к Edge TTS: {e}"))?;

    let (mut socket, _response) =
        tungstenite::connect(request).map_err(|e| format!("Edge TTS не ответил: {e}"))?;

    // Заголовок SSML должен быть в base64 от UTF-8 — так требует сервис
    // для длинных фраз, но и для коротких формат тот же.
    let config = format!(
        "X-Timestamp:{}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n\
         {{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\
         \"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"false\"}},\
         \"outputFormat\":\"{OUTPUT_FORMAT}\"}}}}}}}}",
        timestamp()
    );
    socket
        .send(Message::Text(config.into()))
        .map_err(|e| format!("Edge TTS: не отправила конфиг ({e})"))?;

    let ssml = format!(
        "X-RequestId:{}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{}Z\r\nPath:ssml\r\n\r\n\
         <speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='ru-RU'>\
         <voice name='{voice}'><prosody pitch='{pitch}' rate='{rate}' volume='+0%'>{}</prosody></voice></speak>",
        request_id(),
        timestamp(),
        escape_xml(text)
    );
    socket
        .send(Message::Text(ssml.into()))
        .map_err(|e| format!("Edge TTS: не отправила текст ({e})"))?;

    let mut audio: Vec<u8> = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(45);
    loop {
        if std::time::Instant::now() > deadline {
            return Err("Edge TTS не успел ответить".into());
        }
        match socket.read() {
            Ok(Message::Binary(data)) => {
                // Первые два байта — длина заголовка, потом сам заголовок.
                if data.len() < 2 {
                    continue;
                }
                let header_len = u16::from_be_bytes([data[0], data[1]]) as usize;
                let start = 2 + header_len;
                if start < data.len() {
                    audio.extend_from_slice(&data[start..]);
                }
            }
            Ok(Message::Text(text)) => {
                if text.as_str().contains("Path:turn.end") {
                    break;
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(payload)) => {
                let _ = socket.send(Message::Pong(payload));
            }
            Ok(_) => {}
            Err(e) => {
                if audio.is_empty() {
                    return Err(format!("Edge TTS прервал связь: {e}"));
                }
                break;
            }
        }
    }
    let _ = socket.close(None);
    if audio.is_empty() {
        return Err("Edge TTS вернул пустой звук".into());
    }
    Ok(audio)
}

/// Список голосов Edge для русского языка (для окна настроек).
pub fn russian_voices() -> Vec<(&'static str, &'static str)> {
    vec![
        ("ru-RU-SvetlanaNeural", "Светлана — женский, живой"),
        ("ru-RU-DariyaNeural", "Дария — женский, мягкий"),
        ("ru-RU-SvetlanaDialogueNeural", "Светлана — разговорный"),
        ("ru-RU-DmitryNeural", "Дмитрий — мужской"),
    ]
}

/// Кодировать строку в base64 — пригодится для заголовков.
pub fn base64_encode(text: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_edge_timestamp() {
        let stamp = timestamp();
        assert!(stamp.contains("GMT+0000"));
        assert!(stamp.contains(':'));
        assert!(stamp.len() > 25);
    }

    #[test]
    fn converts_days_to_date() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    }

    #[test]
    fn escapes_ssml() {
        assert_eq!(escape_xml("a & b < c"), "a &amp; b &lt; c");
    }

    #[test]
    fn gec_token_is_uppercase_hex() {
        let token = gec_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(token.chars().all(|c| !c.is_ascii_lowercase()));
    }

    #[test]
    fn gec_token_rounds_to_five_minutes() {
        // Начало пятиминутного окна — подпись не меняется внутри него.
        let base: u64 = 1_700_000_100;
        assert_eq!(gec_token_at(base), gec_token_at(base + 120));
        assert_ne!(gec_token_at(base), gec_token_at(base + 400));
    }
}
