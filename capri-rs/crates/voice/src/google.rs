//! Запасное распознавание через Google Speech API.
//!
//! Используется, когда Vosk не уверен, а фраза слишком короткая: интернет
//! слышит лучше маленькой офлайн-модели.

use std::time::Duration;

pub const SAMPLE_RATE: u32 = 16_000;

/// Распознать 16-битный моно-PCM. `None` — сеть не ответила или не разобрала.
pub fn recognize(pcm: &[i16], language: &str) -> Option<String> {
    if pcm.is_empty() {
        return None;
    }
    let url = format!(
        "https://www.google.com/speech-api/v1/recognize?client=chromium&lang={language}&maxresults=1"
    );
    let body: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
    let response = ureq::post(&url)
        .header("Content-Type", "audio/l16; rate=16000")
        .config()
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .send(body);
    let mut response = response.ok()?;
    let value: serde_json::Value = response.body_mut().read_json().ok()?;
    let text = value
        .get("hypotheses")?
        .as_array()?
        .first()?
        .get("utterance")?
        .as_str()?
        .trim()
        .to_string();
    if text.is_empty() || text == "%s" {
        None
    } else {
        Some(text)
    }
}

/// Стоит ли идти в Google: короткие или совсем неуверенные фразы.
pub fn worth_trying(text: &str) -> bool {
    let words = text.split_whitespace().count();
    text.trim().is_empty() || words <= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_phrases_go_to_google() {
        assert!(worth_trying(""));
        assert!(worth_trying("капри"));
        assert!(!worth_trying("открой пожалуйста телеграм и напиши маме"));
    }

    #[test]
    fn empty_audio_is_skipped() {
        assert!(recognize(&[], "ru-RU").is_none());
    }
}
