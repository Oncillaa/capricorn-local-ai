//! Голос Капри: Edge TTS с кэшем, эффекты, запасной голос Windows.
//!
//! Перенос класса `Speaker` из `voice.py`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;

use capri_core::Config;
use parking_lot::Mutex;

use crate::audio;
use crate::edge;

/// Сколько символов во фразе ещё имеет смысл кэшировать.
const CACHE_MAX_TEXT: usize = 160;
/// Сколько файлов держим в кэше.
const CACHE_MAX_FILES: usize = 600;

pub struct Speaker {
    voice: String,
    rate: String,
    pitch: String,
    fx: f32,
    lively: bool,
    shift: f32,
    engine: String,
    volume: AtomicU8,
    stop: Arc<AtomicBool>,
    cache_dir: PathBuf,
    last_error: Mutex<Option<String>>,
}

impl Speaker {
    pub fn new(cfg: &Config, cache_root: &Path) -> Self {
        let cache_dir = cache_root.join("tts-cache");
        let _ = std::fs::create_dir_all(&cache_dir);
        // Громкость 0 — это почти всегда старая ошибка (в Python-версии такой
        // ноль попадал в config.json, и Капри говорила в полной тишине).
        // Молчать «по настройке» пользователь может и громкостью 1.
        let volume = if cfg.voice_volume == 0 {
            log::warn!(
                "voice_volume = 0 в config.json — это полная тишина. Говорю на 100, \
                 поставь своё значение, если нужно тише."
            );
            100
        } else {
            cfg.voice_volume
        };
        if cfg.tts_engine != "edge" && cfg.tts_engine != "sapi" {
            log::warn!(
                "tts_engine = «{}» не поддерживается, использую «edge» \
                 (с запасным голосом Windows).",
                cfg.tts_engine
            );
        }
        Self {
            voice: cfg.voice.clone(),
            rate: cfg.voice_rate.clone(),
            pitch: cfg.voice_pitch.clone(),
            fx: cfg.voice_fx,
            lively: cfg.voice_lively,
            shift: cfg.voice_shift,
            // Неизвестный движок ведёт себя как edge, а не молчит.
            engine: if cfg.tts_engine == "sapi" { "sapi".to_string() } else { "edge".to_string() },
            volume: AtomicU8::new(volume),
            stop: Arc::new(AtomicBool::new(false)),
            cache_dir,
            last_error: Mutex::new(None),
        }
    }

    /// Произнести текст. Блокирует поток до конца фразы.
    /// `true` — речь прервали (пользователь перебил).
    pub fn say(&self, text: &str) -> bool {
        self.stop.store(false, Ordering::Relaxed);
        let text = clean_for_speech(text);
        if text.is_empty() {
            return false;
        }
        let volume = self.volume.load(Ordering::Relaxed);

        // 1. Готовый кэш.
        let cached = self.cache_path(&text);
        if cached.is_file() {
            touch(&cached);
            if let Ok(samples) = std::fs::read(&cached).map(|b| audio::decode_bytes(&b, audio::FX_SR)) {
                if let Ok(samples) = samples {
                    if !samples.is_empty() {
                        return audio::play_samples(samples, audio::FX_SR, volume);
                    }
                }
            }
        }

        // 2. Edge TTS.
        if self.engine != "sapi" {
            match self.render_edge(&text) {
                Ok(samples) if !samples.is_empty() => {
                    let processed = if self.fx > 0.0 {
                        audio::apply_voice_fx(&samples, audio::FX_SR, self.fx)
                    } else {
                        samples
                    };
                    if text.chars().count() <= CACHE_MAX_TEXT {
                        self.store_cache(&text, &processed);
                    }
                    return audio::play_samples(processed, audio::FX_SR, volume);
                }
                Ok(_) => {
                    self.set_error("Edge TTS вернул пустой звук");
                }
                Err(e) => {
                    *self.last_error.lock() = Some(e.clone());
                    log::info!("edge-tts недоступен ({e}), использую голос Windows");
                }
            }
        }

        // 3. Голос Windows.
        self.speak_sapi(&text, volume)
    }

    /// Заготовить фразы заранее, чтобы они звучали мгновенно.
    pub fn prewarm(&self, texts: &[String]) {
        for text in texts {
            let text = clean_for_speech(text);
            if text.is_empty() || text.chars().count() > CACHE_MAX_TEXT {
                continue;
            }
            let path = self.cache_path(&text);
            if path.is_file() {
                continue;
            }
            match self.render_edge(&text) {
                Ok(samples) if !samples.is_empty() => {
                    let processed = if self.fx > 0.0 {
                        audio::apply_voice_fx(&samples, audio::FX_SR, self.fx)
                    } else {
                        samples
                    };
                    self.store_cache(&text, &processed);
                }
                _ => return,
            }
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        audio::stop_playback();
    }

    pub fn set_volume(&self, percent: u8) {
        self.volume.store(percent.min(100), Ordering::Relaxed);
    }

    pub fn error(&self) -> Option<String> {
        self.last_error.lock().clone()
    }

    fn set_error(&self, text: &str) {
        *self.last_error.lock() = Some(text.to_string());
    }

    /// Синтез с «живой» интонацией: по куску на каждое предложение,
    /// со своим сдвигом высоты, склейка через короткие паузы.
    fn render_edge(&self, text: &str) -> Result<Vec<f32>, String> {
        let sentences = split_sentences(text);
        let semitones = lively_semitones(&sentences, self.shift, self.lively);
        let base_rate = percent_of(&self.rate);

        let mut chunks: Vec<f32> = Vec::new();
        let gap = vec![0.0f32; (audio::FX_SR as f32 * 0.13) as usize];
        let mut any = false;

        for (sentence, semis) in sentences.iter().zip(semitones.iter()) {
            // Компенсируем сдвиг высоты темпом, чтобы фраза не «жевала».
            let adjusted = base_rate - ((2f32.powf(semis / 12.0) - 1.0) * 100.0);
            let rate = format!("{:+}%", adjusted.round() as i32);
            let mp3 = edge::synthesize(sentence, &self.voice, &rate, &self.pitch)?;
            let Ok(decoded) = audio::decode_bytes(&mp3, audio::FX_SR) else {
                continue;
            };
            let trimmed = audio::trim_silence(&decoded, audio::FX_SR, 0.012);
            if trimmed.is_empty() {
                continue;
            }
            let shifted = audio::pitch_shift(&trimmed, *semis);
            chunks.extend(shifted);
            chunks.extend(&gap);
            any = true;
        }

        if !any {
            // Не вышло по предложениям — синтезируем фразу целиком.
            let mp3 = edge::synthesize(text, &self.voice, &self.rate, &self.pitch)?;
            return audio::decode_bytes(&mp3, audio::FX_SR);
        }
        Ok(chunks)
    }

    fn cache_path(&self, text: &str) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        self.engine.hash(&mut hasher);
        self.voice.hash(&mut hasher);
        self.rate.hash(&mut hasher);
        self.pitch.hash(&mut hasher);
        self.fx.to_bits().hash(&mut hasher);
        self.volume.load(Ordering::Relaxed).hash(&mut hasher);
        self.lively.hash(&mut hasher);
        self.shift.to_bits().hash(&mut hasher);
        self.cache_dir.join(format!("{:016x}.wav", hasher.finish()))
    }

    fn store_cache(&self, text: &str, samples: &[f32]) {
        let path = self.cache_path(text);
        let volume = self.volume.load(Ordering::Relaxed);
        if audio::write_wav(&path, samples, audio::FX_SR, volume).is_ok() {
            trim_cache(&self.cache_dir);
        }
    }

    /// Голос Windows через SAPI.
    fn speak_sapi(&self, text: &str, volume: u8) -> bool {
        match sapi_speak(text, volume, &self.stop) {
            Ok(interrupted) => interrupted,
            Err(e) => {
                log::warn!("Голос Windows не сработал: {e}");
                self.set_error(&e);
                false
            }
        }
    }
}

/// Убрать из текста то, что нельзя произнести.
pub fn clean_for_speech(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        let lower = word.to_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.") {
            continue;
        }
        out.push_str(word);
        out.push(' ');
    }
    let stripped: String = out
        .chars()
        .filter(|c| !matches!(c, '*' | '#' | '_' | '`' | '>' | '|' | '[' | ']'))
        .collect();
    let mut result = String::with_capacity(stripped.len());
    let mut last_space = false;
    for ch in stripped.chars() {
        if ch.is_whitespace() {
            if !last_space {
                result.push(' ');
                last_space = true;
            }
        } else {
            result.push(ch);
            last_space = false;
        }
    }
    result.trim().to_string()
}

/// Разбить текст на предложения.
fn split_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, ch) in chars.iter().enumerate() {
        current.push(*ch);
        let is_end = matches!(ch, '.' | '!' | '?' | '…');
        let next_space = chars.get(i + 1).map(|c| c.is_whitespace()).unwrap_or(true);
        if is_end && next_space {
            let trimmed = current.trim().to_string();
            if !trimmed.is_empty() {
                sentences.push(trimmed);
            }
            current.clear();
        }
    }
    let tail = current.trim().to_string();
    if !tail.is_empty() {
        sentences.push(tail);
    }
    if sentences.is_empty() {
        vec![text.trim().to_string()]
    } else {
        sentences
    }
}

/// Сдвиг высоты на каждое предложение — от него речь перестаёт быть роботом.
fn lively_semitones(sentences: &[String], base: f32, lively: bool) -> Vec<f32> {
    use rand::Rng;
    let mut rng = rand::rng();
    sentences
        .iter()
        .map(|sentence| {
            let mut k = base;
            if lively {
                k += rng.random_range(-2.5f32..3.0f32);
                let end = sentence.trim_end();
                if end.ends_with('!') {
                    k += 4.0;
                } else if end.ends_with('?') {
                    k += 3.0;
                } else if end.ends_with('…') || end.ends_with("...") {
                    k -= 2.5;
                }
            }
            k
        })
        .collect()
}

/// «+5%» → 5.0
fn percent_of(value: &str) -> f32 {
    let digits: String = value
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '-' || *c == '+')
        .collect();
    digits.parse().unwrap_or(0.0)
}

fn touch(path: &Path) {
    // Обновляем mtime, чтобы старые файлы не вытесняли свежие.
    if let Ok(file) = std::fs::OpenOptions::new().append(true).open(path) {
        let _ = file.set_len(std::fs::metadata(path).map(|m| m.len()).unwrap_or(0));
    }
}

/// Держать кэш в разумных размерах: удаляем самые старые файлы.
fn trim_cache(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();
    if files.len() <= CACHE_MAX_FILES {
        return;
    }
    files.sort_by_key(|(time, _)| *time);
    let excess = files.len() - CACHE_MAX_FILES;
    for (_, path) in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}

/// Голос Windows (SAPI) — работает всегда, даже без интернета.
fn sapi_speak(text: &str, volume: u8, stop: &Arc<AtomicBool>) -> Result<bool, String> {
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::Media::Speech::{
            ISpVoice, SPVOICESTATUS, SPF_ASYNC, SPF_PURGEBEFORESPEAK,
        };
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
        };

        // {96749377-3391-11D2-9EE3-00C04F797396} — класс голоса Windows (SpVoice).
        const CLSID_SPVOICE: windows::core::GUID =
            windows::core::GUID::from_u128(0x96749377_3391_11D2_9EE3_00C04F797396);
        // SPF_ASYNC | SPF_PURGEBEFORESPEAK: не блокирует поток и перебивает прошлую фразу.
        let flags = (SPF_ASYNC.0 | SPF_PURGEBEFORESPEAK.0) as u32;

        /// Строка в UTF-16 с завершающим нулём — так ждёт SAPI.
        fn wide(text: &str) -> Vec<u16> {
            text.encode_utf16().chain(std::iter::once(0)).collect()
        }

        struct Com;
        impl Drop for Com {
            fn drop(&mut self) {
                unsafe { CoUninitialize() };
            }
        }

        unsafe {
            let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            if hr.is_err() {
                return Err(format!("COM не поднялся: {hr:?}"));
            }
            let _com = Com;

            let voice: ISpVoice = CoCreateInstance(&CLSID_SPVOICE, None, CLSCTX_ALL)
                .map_err(|e| format!("SAPI недоступен: {e}"))?;
            voice
                .SetVolume(volume.min(100) as u16)
                .map_err(|e| format!("SAPI: громкость не ставится: {e}"))?;

            let phrase = wide(text);
            voice
                .Speak(PCWSTR(phrase.as_ptr()), flags, None)
                .map_err(|e| format!("SAPI: не смог заговорить: {e}"))?;

            let silence = wide("");
            loop {
                if stop.load(Ordering::Relaxed) {
                    // Пустая фраза с PURGE — заткнуться немедленно.
                    let _ = voice.Speak(PCWSTR(silence.as_ptr()), SPF_PURGEBEFORESPEAK.0 as u32, None);
                    return Ok(true);
                }
                let mut status = SPVOICESTATUS::default();
                if voice.GetStatus(&mut status, std::ptr::null_mut()).is_err() {
                    return Ok(false);
                }
                // 1 — STRS_DONE.
                if status.dwRunningState == 1 {
                    return Ok(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (text, volume, stop);
        Err("голос Windows доступен только на Windows".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_text_for_speech() {
        let cleaned = clean_for_speech("Смотри https://example.com **вот** `это`");
        assert!(!cleaned.contains("http"));
        assert!(!cleaned.contains('*'));
        assert!(cleaned.contains("вот"));
    }

    #[test]
    fn splits_into_sentences() {
        let parts = split_sentences("Привет. Как дела? Хорошо!");
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], "Привет.");
    }

    #[test]
    fn reads_rate_percent() {
        assert_eq!(percent_of("+5%"), 5.0);
        assert_eq!(percent_of("-10%"), -10.0);
        assert_eq!(percent_of(""), 0.0);
    }

    #[test]
    fn lively_shift_follows_punctuation() {
        let sentences = vec!["Стой!".to_string(), "Готово.".to_string()];
        // Без «живости» высота ровная.
        assert_eq!(lively_semitones(&sentences, 0.0, false), vec![0.0, 0.0]);
        // С «живостью» восклицание уходит вверх, а точка — нет.
        let lively = lively_semitones(&sentences, 0.0, true);
        assert!((1.5..=7.0).contains(&lively[0]), "{lively:?}");
        assert!((-2.5..=3.0).contains(&lively[1]), "{lively:?}");
    }
}
