//! Голос Капри: Edge TTS с кэшем, эффекты, запасной голос Windows.
//!
//! Перенос класса `Speaker` из `voice.py`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use capri_core::Config;
use parking_lot::Mutex;

use crate::audio;
use crate::edge;

/// Сколько символов во фразе ещё имеет смысл кэшировать.
const CACHE_MAX_TEXT: usize = 160;
/// Сколько файлов держим в кэше.
const CACHE_MAX_FILES: usize = 600;
/// Сколько не трогать edge-tts после сетевого отказа.
///
/// Без этого каждая фраза сначала ждала ответа сети (на плохом канале — секунды),
/// и всё это время Капри стояла как вкопанная. Лучше сразу говорить другим
/// голосом, а сеть пощупать через десять минут.
const EDGE_RETRY_AFTER: Duration = Duration::from_secs(600);

/// Голосовой профиль целиком: меняется из настроек на лету.
///
/// Держим одним куском под мьютексом, а не шестью отдельными атомиками:
/// иначе панель могла бы выставить новый голос со старой скоростью.
#[derive(Clone, Debug)]
pub struct VoiceProfile {
    pub voice: String,
    pub rate: String,
    pub pitch: String,
    pub lively: bool,
    pub shift: f32,
    pub engine: String,
}

pub struct Speaker {
    /// Голос, скорость, тембр, живость и движок.
    profile: Mutex<VoiceProfile>,
    /// Сила цифровых эффектов голоса (атомарно: меняется из панели на лету).
    fx: AtomicU32,
    volume: AtomicU8,
    stop: Arc<AtomicBool>,
    cache_dir: PathBuf,
    /// Где SAPI может писать промежуточный WAV.
    ///
    /// Ей нельзя во всё подряд: на этой машине `ISpeechFileStream` отказывается
    /// открывать файлы вне каталога программы (`0x80070057`), хотя обычная
    /// запись туда работает. Поэтому сначала каталог программы, потом кэш.
    work_dirs: Vec<PathBuf>,
    last_error: Mutex<Option<String>>,
    /// До какого момента edge-tts считается недоступным.
    edge_down_until: Mutex<Option<Instant>>,
    /// До какого момента не пробуем сетевой голос через Python.
    py_down_until: Mutex<Option<Instant>>,
}

impl Speaker {
    /// `cache_root` — где держать кэш озвучки, `work_dir` — каталог программы
    /// (там SAPI пишет промежуточный WAV; `None` — только кэш).
    pub fn new(cfg: &Config, cache_root: &Path, work_dir: Option<&Path>) -> Self {
        // Кэш по умолчанию в `%LOCALAPPDATA%\Capri`, но писать туда можно не
        // всегда (на этой машине — «отказано в доступе»). Тогда держим кэш
        // рядом с программой: озвучка не должна зависеть от прав на AppData.
        let mut cache_dir = cache_root.join("tts-cache");
        if !dir_writable(&cache_dir) {
            if let Some(dir) = work_dir {
                let alternative = dir.join("tts-cache");
                if dir_writable(&alternative) {
                    log::info!(
                        "Кэш озвучки: {} недоступен, держу рядом с программой ({})",
                        cache_dir.display(),
                        alternative.display()
                    );
                    cache_dir = alternative;
                }
            }
        }
        let mut work_dirs: Vec<PathBuf> = Vec::new();
        if let Some(dir) = work_dir {
            work_dirs.push(dir.to_path_buf());
        }
        work_dirs.push(cache_dir.clone());
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
        // Конфиг чинит незнакомый движок сам (`Config::load`), но сюда профиль
        // может прийти и напрямую — например, из панели настроек. Тогда честно
        // предупреждаем один раз и говорим через edge, а не молчим.
        if !capri_core::config::TTS_ENGINES.contains(&cfg.tts_engine.as_str()) {
            log::warn!(
                "tts_engine = «{}» не поддерживается, использую «edge» \
                 (с запасным голосом Windows).",
                cfg.tts_engine
            );
        }
        Self {
            profile: Mutex::new(VoiceProfile {
                voice: cfg.voice.clone(),
                rate: cfg.voice_rate.clone(),
                pitch: cfg.voice_pitch.clone(),
                lively: cfg.voice_lively,
                shift: cfg.voice_shift,
                // Неизвестный движок ведёт себя как edge, а не молчит.
                engine: if cfg.tts_engine == "sapi" {
                    "sapi".to_string()
                } else {
                    "edge".to_string()
                },
            }),
            fx: AtomicU32::new(cfg.voice_fx.to_bits()),
            volume: AtomicU8::new(volume),
            stop: Arc::new(AtomicBool::new(false)),
            cache_dir,
            work_dirs,
            last_error: Mutex::new(None),
            edge_down_until: Mutex::new(None),
            py_down_until: Mutex::new(None),
        }
    }

    /// Текущий голосовой профиль (копия — чтобы не держать мьютекс).
    pub fn profile(&self) -> VoiceProfile {
        self.profile.lock().clone()
    }

    /// Применить настройки голоса из конфига: панель правит файл, а голос
    /// подхватывает изменения без перезапуска.
    pub fn apply_profile(&self, cfg: &Config) {
        let fresh = VoiceProfile {
            voice: cfg.voice.clone(),
            rate: cfg.voice_rate.clone(),
            pitch: cfg.voice_pitch.clone(),
            lively: cfg.voice_lively,
            shift: cfg.voice_shift,
            engine: if cfg.tts_engine == "sapi" {
                "sapi".to_string()
            } else {
                "edge".to_string()
            },
        };
        {
            let mut guard = self.profile.lock();
            if guard.voice != fresh.voice
                || guard.rate != fresh.rate
                || guard.pitch != fresh.pitch
                || guard.lively != fresh.lively
                || (guard.shift - fresh.shift).abs() > f32::EPSILON
                || guard.engine != fresh.engine
            {
                log::info!(
                    "Голос обновился: {} · скорость {} · тембр {} · {}",
                    fresh.voice,
                    fresh.rate,
                    fresh.pitch,
                    if fresh.engine == "sapi" { "голос Windows" } else { "сетевой" },
                );
            }
            *guard = fresh;
        }
        self.set_fx(cfg.voice_fx);
        self.set_volume(cfg.voice_volume);
    }

    /// Сила эффектов голоса, 0.0…1.0.
    pub fn set_fx(&self, value: f32) {
        let clamped = value.clamp(0.0, 1.0);
        self.fx.store(clamped.to_bits(), Ordering::Relaxed);
    }

    fn fx_value(&self) -> f32 {
        f32::from_bits(self.fx.load(Ordering::Relaxed))
    }

    /// Можно ли сейчас пробовать сетевой голос.
    pub fn edge_available(&self) -> bool {
        match *self.edge_down_until.lock() {
            Some(until) => Instant::now() >= until,
            None => true,
        }
    }

    /// Можно ли пробовать сетевой голос через Python-пакет.
    fn py_edge_available(&self) -> bool {
        match *self.py_down_until.lock() {
            Some(until) => Instant::now() >= until,
            None => true,
        }
    }

    /// Python-путь сработал — значит, сеть жива.
    fn py_edge_ok(&self) {
        if self.py_down_until.lock().is_some() {
            log::info!("Сетевой голос через Python снова работает");
        }
        *self.py_down_until.lock() = None;
    }

    /// Python-путь не сработал: не долбим его каждую фразу.
    fn py_edge_failed(&self, why: &str) {
        let first = self.py_down_until.lock().is_none();
        *self.py_down_until.lock() = Some(Instant::now() + EDGE_RETRY_AFTER);
        if first {
            log::info!(
                "Сетевой голос через Python не сработал ({why}) — \
                 {} минут голос Windows",
                EDGE_RETRY_AFTER.as_secs() / 60
            );
        } else {
            log::debug!("Сетевой голос через Python всё ещё не работает ({why})");
        }
    }

    /// Сеть подвела — запоминаем и не дёргаем её какое-то время.
    fn edge_failed(&self, why: &str) {
        let first = self.edge_down_until.lock().is_none();
        *self.edge_down_until.lock() = Some(Instant::now() + EDGE_RETRY_AFTER);
        *self.last_error.lock() = Some(why.to_string());
        if first {
            log::info!(
                "edge-tts недоступен ({why}) — {} минут говорю запасным голосом",
                EDGE_RETRY_AFTER.as_secs() / 60
            );
        } else {
            log::debug!("edge-tts всё ещё недоступен ({why})");
        }
    }

    /// Сеть ответила — снимаем запрет.
    fn edge_ok(&self) {
        if self.edge_down_until.lock().is_some() {
            log::info!("edge-tts снова отвечает — возвращаюсь к обычному голосу");
        }
        *self.edge_down_until.lock() = None;
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

        // 2. Сетевой голос. Первым — Python-пакет `edge-tts`: у него
        //    актуальный токен службы, а наш прямой синтез в этой сети получает
        //    403 и тратит на это секунды. Свой синтез остаётся для машин, где
        //    питона нет.
        if self.profile().engine != "sapi" && self.py_edge_available() {
            match crate::py_edge::synthesize(&text, &self.profile().voice, &self.profile().rate, &self.profile().pitch, &self.work_dirs)
            {
                Ok(bytes) => {
                    self.py_edge_ok();
                    if let Some(interrupted) = self.finish_render(&text, bytes, volume) {
                        return interrupted;
                    }
                }
                Err(e) => {
                    self.py_edge_failed(&e);
                }
            }
        }

        // 2.5. Свой синтез через Edge TTS (если сеть не подводила).
        if self.profile().engine != "sapi" && self.edge_available() {
            match self.render_edge(&text) {
                Ok(samples) if !samples.is_empty() => {
                    self.edge_ok();
                    let processed = if self.fx_value() > 0.0 {
                        audio::apply_voice_fx(&samples, audio::FX_SR, self.fx_value())
                    } else {
                        samples
                    };
                    if text.chars().count() <= CACHE_MAX_TEXT {
                        self.store_cache(&text, &processed);
                    }
                    self.clear_error();
                    return audio::play_samples(processed, audio::FX_SR, volume);
                }
                Ok(_) => {
                    self.edge_failed("Edge TTS вернул пустой звук");
                }
                Err(e) => {
                    self.edge_failed(&e);
                }
            }
        }

        // 3. Голос Windows.
        self.speak_sapi(&text, volume)
    }

    /// Заготовить фразы заранее, чтобы они звучали мгновенно.
    pub fn prewarm(&self, texts: &[String]) {
        if !self.edge_available() {
            return;
        }
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
                    self.edge_ok();
                    let processed = if self.fx_value() > 0.0 {
                        audio::apply_voice_fx(&samples, audio::FX_SR, self.fx_value())
                    } else {
                        samples
                    };
                    self.store_cache(&text, &processed);
                }
                Ok(_) => {
                    self.edge_failed("Edge TTS вернул пустой звук");
                    return;
                }
                Err(e) => {
                    self.edge_failed(&e);
                    return;
                }
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

    /// Забыть прошлую ошибку: голос ведь получился.
    fn clear_error(&self) {
        *self.last_error.lock() = None;
    }

    /// Синтез с «живой» интонацией: по куску на каждое предложение,
    /// со своим сдвигом высоты, склейка через короткие паузы.
    fn render_edge(&self, text: &str) -> Result<Vec<f32>, String> {
        let sentences = split_sentences(text);
        let semitones = lively_semitones(&sentences, self.profile().shift, self.profile().lively);
        let base_rate = percent_of(&self.profile().rate);

        let mut chunks: Vec<f32> = Vec::new();
        let gap = vec![0.0f32; (audio::FX_SR as f32 * 0.13) as usize];
        let mut any = false;

        for (sentence, semis) in sentences.iter().zip(semitones.iter()) {
            // Компенсируем сдвиг высоты темпом, чтобы фраза не «жевала».
            let adjusted = base_rate - ((2f32.powf(semis / 12.0) - 1.0) * 100.0);
            let rate = format!("{:+}%", adjusted.round() as i32);
            let mp3 = edge::synthesize(sentence, &self.profile().voice, &rate, &self.profile().pitch)?;
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
            let mp3 = edge::synthesize(text, &self.profile().voice, &self.profile().rate, &self.profile().pitch)?;
            return audio::decode_bytes(&mp3, audio::FX_SR);
        }
        Ok(chunks)
    }

    fn cache_path(&self, text: &str) -> PathBuf {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        self.profile().engine.hash(&mut hasher);
        self.profile().voice.hash(&mut hasher);
        self.profile().rate.hash(&mut hasher);
        self.profile().pitch.hash(&mut hasher);
        self.fx_value().to_bits().hash(&mut hasher);
        self.volume.load(Ordering::Relaxed).hash(&mut hasher);
        self.profile().lively.hash(&mut hasher);
        self.profile().shift.to_bits().hash(&mut hasher);
        self.cache_dir.join(format!("{:016x}.wav", hasher.finish()))
    }

    fn store_cache(&self, text: &str, samples: &[f32]) {
        let path = self.cache_path(text);
        let volume = self.volume.load(Ordering::Relaxed);
        match audio::write_wav(&path, samples, audio::FX_SR, volume) {
            Ok(()) => trim_cache(&self.cache_dir),
            Err(e) => log::warn!("Кэш озвучки не записался ({}): {e}", path.display()),
        }
    }

    /// Голос Windows (SAPI): сначала синтез в файл, потом своё проигрывание.
    ///
    /// У SAPI бывает пустая категория звуковых выходов (на этой машине —
    /// совсем пустая), и тогда `Speak` в устройство сразу падает. Синтез в файл
    /// работает без звукового устройства, а играем и обрабатываем эффектами мы
    /// сами — тем же путём, что и сетевой голос.
    fn speak_sapi(&self, text: &str, volume: u8) -> bool {
        let russian = self.profile().voice.to_lowercase().contains("ru");
        let mut rendered: Option<(std::path::PathBuf, Vec<u8>)> = None;
        let mut problems: Vec<String> = Vec::new();
        for dir in &self.work_dirs {
            if let Err(e) = std::fs::create_dir_all(dir) {
                problems.push(format!("{}: каталог не создался ({e})", dir.display()));
                continue;
            }
            let path = dir.join("sapi-phrase.wav");
            let _ = std::fs::remove_file(&path);
            match sapi_render_wav(text, russian, volume, &path) {
                Ok(()) => match std::fs::read(&path) {
                    Ok(bytes) if bytes.len() > 64 => {
                        rendered = Some((path, bytes));
                        break;
                    }
                    Ok(bytes) => problems.push(format!(
                        "{}: запись пустая ({} байт)",
                        path.display(),
                        bytes.len()
                    )),
                    Err(e) => problems.push(format!("{}: не прочиталась ({e})", path.display())),
                },
                Err(e) => {
                    problems.push(format!("{}: {e}", path.display()));
                    let _ = std::fs::remove_file(&path);
                }
            }
        }

        let Some((path, bytes)) = rendered else {
            // Прямые вызовы SAPI из нашего процесса не сработали — это не
            // приговор: тот же SAPI из PowerShell пишет нормальную запись.
            // Синтез в отдельном процессе медленнее, но лучше молчания.
            log::debug!("SAPI из процесса не смог ({}), пробую PowerShell", problems.join("; "));
            match crate::ps_sapi::render(text, &self.work_dirs) {
                Ok(bytes) => {
                    if let Some(samples) = self.finish_render(text, bytes, volume) {
                        return samples;
                    }
                    return false;
                }
                Err(e) => {
                    let report = problems.join("; ");
                    log::warn!("Голос Windows не сработал: {report}; через PowerShell: {e}");
                    let reason = problems
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| "SAPI молчит".into());
                    self.set_error(&reason);
                    return false;
                }
            }
        };
        let _ = std::fs::remove_file(&path);

        match self.finish_render(text, bytes, volume) {
            Some(interrupted) => interrupted,
            None => false,
        }
    }

    /// Декодировать готовую запись, приправить эффектами и проиграть.
    ///
    /// `Some(interrupted)` — проиграли (и, может, перебили), `None` — запись
    /// не разобралась.
    fn finish_render(&self, text: &str, bytes: Vec<u8>, volume: u8) -> Option<bool> {
        let samples = match audio::decode_bytes(&bytes, audio::FX_SR) {
            Ok(samples) => samples,
            Err(e) => {
                log::warn!("Голос Windows: не разобрала запись ({e})");
                return None;
            }
        };
        if samples.is_empty() {
            return None;
        }
        let processed = if self.fx_value() > 0.0 {
            audio::apply_voice_fx(&samples, audio::FX_SR, self.fx_value())
        } else {
            samples
        };
        if text.chars().count() <= CACHE_MAX_TEXT {
            self.store_cache(text, &processed);
        }
        self.clear_error();
        Some(audio::play_samples(processed, audio::FX_SR, volume))
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

/// Можно ли писать в каталог: создаём его и пробуем записать файл.
fn dir_writable(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".capri-write-test");
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn touch(path: &Path) {    // Обновляем mtime, чтобы старые файлы не вытесняли свежие.
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

/// Голос Windows (SAPI) — синтез в WAV-файл, без звукового устройства.
///
/// Идём тем же путём, что PowerShell: объект `SAPI.SpVoice` + `SpFileStream`.
/// Прямые вызовы `ISpVoice` в этом окружении не работают (у голоса нет выхода),
/// а автоматизационные интерфейсы — работают.
#[cfg(windows)]
fn sapi_render_wav(
    text: &str,
    russian: bool,
    volume: u8,
    path: &Path,
) -> Result<(), String> {
    use windows::core::GUID;
    use windows::Win32::Foundation::{VARIANT_FALSE, VARIANT_TRUE};
    use windows::Win32::Media::Speech::{
        ISpeechFileStream, ISpeechVoice, SSFMCreateForWrite, SVSFDefault,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };

    // {96749377-3391-11D2-9EE3-00C04F797396} — голос Windows (SpVoice).
    const CLSID_SPVOICE: GUID = GUID::from_u128(0x96749377_3391_11D2_9EE3_00C04F797396);
    // {947812B3-2AE1-4644-BA86-9E90DED7EC91} — файловый поток (SpFileStream).
    const CLSID_SPFILESTREAM: GUID = GUID::from_u128(0x947812B3_2AE1_4644_BA86_9E90DED7EC91);

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

        let voice: ISpeechVoice = CoCreateInstance(&CLSID_SPVOICE, None, CLSCTX_ALL)
            .map_err(|e| format!("SAPI недоступен: {e}"))?;
        let stream: ISpeechFileStream = CoCreateInstance(&CLSID_SPFILESTREAM, None, CLSCTX_ALL)
            .map_err(|e| format!("SAPI: файл для записи не создался: {e}"))?;

        // Русский голос, если он в системе есть (804 — код языка ru).
        if russian {
            let wanted = windows::core::BSTR::from("Language=804");
            let empty = windows::core::BSTR::new();
            if let Ok(list) = voice.GetVoices(&wanted, &empty) {
                if list.Count().unwrap_or(0) > 0 {
                    if let Ok(token) = list.Item(0) {
                        let _ = voice.putref_Voice(&token);
                    }
                }
            }
        }
        let _ = voice.SetVolume(volume.min(100) as i32);

        let wide = path.to_string_lossy().to_string();
        stream
            .Open(
                &windows::core::BSTR::from(wide.as_str()),
                SSFMCreateForWrite,
                VARIANT_FALSE,
            )
            .map_err(|e| format!("SAPI: файл не открылся: {e}"))?;
        let _ = voice.SetAllowAudioOutputFormatChangesOnNextSet(VARIANT_TRUE);
        voice
            .putref_AudioOutputStream(&stream)
            .map_err(|e| format!("SAPI: запись в файл не принята: {e}"))?;
        voice
            .Speak(&windows::core::BSTR::from(text), SVSFDefault)
            .map_err(|e| format!("SAPI: не смог заговорить: {e}"))?;
        let _ = stream.Close();
        Ok(())
    }
}

#[cfg(not(windows))]
fn sapi_render_wav(_text: &str, _russian: bool, _volume: u8, _path: &Path) -> Result<(), String> {
    Err("голос Windows доступен только на Windows".into())
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
