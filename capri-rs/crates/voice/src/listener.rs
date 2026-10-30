//! Микрофон и распознавание речи: cpal + Vosk, загруженный на лету.
//!
//! `libvosk.dll` рядом с exe не обязателен: если его нет, Капри честно
//! говорит, что не слышит (ошибка `no_model`), но не падает.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Sample;
use parking_lot::Mutex;

use capri_core::Config;

/// Частота, которую ждёт Vosk.
pub const RATE: u32 = 16_000;
/// Усиление уровня для индикатора в интерфейсе.
const LEVEL_SCALE: f32 = 6.0;
const GATE_MAX: f32 = 0.35;

pub type TextCallback = Arc<dyn Fn(&str) + Send + Sync>;
/// Готовая фраза вместе со звуком (16 кГц, моно) — звук нужен,
/// чтобы при желании перепроверить фразу через Google.
pub type FinalCallback = Arc<dyn Fn(&str, &[i16]) + Send + Sync>;
pub type ErrorCallback = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Clone)]
pub struct ListenerOptions {
    pub model_dir: PathBuf,
    pub device: Option<String>,
    pub gain: f32,
    pub sensitivity: i32,
}

pub struct ListenerHandle {
    pub muted: Arc<AtomicBool>,
    pub reset: Arc<AtomicBool>,
    pub level: Arc<AtomicU32>,
    device: Arc<Mutex<Option<String>>>,
    gain: Arc<Mutex<f32>>,
    sensitivity: Arc<AtomicI32>,
    active: Arc<Mutex<Option<String>>>,
    switch: Sender<()>,
}

impl ListenerHandle {
    /// Запустить слух. `None` — слушать нечем (нет модели или `libvosk.dll`),
    /// причина уходит в `on_error`.
    pub fn spawn(
        opts: ListenerOptions,
        on_partial: TextCallback,
        on_final: FinalCallback,
        on_error: ErrorCallback,
    ) -> Option<Self> {
        // Модель проверяем заранее — без неё слушать бессмысленно.
        let Some(model_path) = ascii_model_path(&opts.model_dir) else {
            on_error("no_model");
            return None;
        };
        if !model_complete(&model_path) {
            on_error("no_model");
            return None;
        }
        let mut engine = match Vosk::load() {
            Ok(engine) => engine,
            Err(e) => {
                log::warn!("libvosk.dll не загрузилась: {e}");
                on_error("no_model");
                return None;
            }
        };
        if let Err(e) = engine.open_model(&model_path) {
            log::warn!("Модель Vosk не открылась: {e}");
            on_error("model_broken");
            return None;
        }

        let muted = Arc::new(AtomicBool::new(false));
        let reset = Arc::new(AtomicBool::new(false));
        let level = Arc::new(AtomicU32::new(0f32.to_bits()));
        let device = Arc::new(Mutex::new(opts.device.clone()));
        let gain = Arc::new(Mutex::new(opts.gain));
        let sensitivity = Arc::new(AtomicI32::new(opts.sensitivity));
        let active = Arc::new(Mutex::new(None::<String>));
        let (switch_tx, switch_rx) = mpsc::channel::<()>();

        let handle = Self {
            muted: Arc::clone(&muted),
            reset: Arc::clone(&reset),
            level: Arc::clone(&level),
            device: Arc::clone(&device),
            gain: Arc::clone(&gain),
            sensitivity: Arc::clone(&sensitivity),
            active: Arc::clone(&active),
            switch: switch_tx,
        };

        std::thread::spawn(move || {
            let worker = Worker {
                engine,
                on_partial,
                on_final,
                on_error,
                muted,
                reset,
                level,
                device,
                gain,
                sensitivity,
                active,
            };
            worker.run(switch_rx);
        });

        Some(handle)
    }

    /// Переключить микрофон (по имени из списка [`input_devices`]).
    pub fn set_device(&self, name: Option<String>) {
        *self.device.lock() = name.filter(|n| !n.trim().is_empty());
        let _ = self.switch.send(());
    }

    pub fn set_gain(&self, gain: f32) {
        *self.gain.lock() = gain.clamp(0.1, 8.0);
    }

    pub fn set_sensitivity(&self, value: i32) {
        self.sensitivity.store(value.clamp(0, 100), Ordering::Relaxed);
    }

    pub fn active_name(&self) -> Option<String> {
        self.active.lock().clone()
    }

    pub fn is_muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub fn set_muted(&self, value: bool) {
        self.muted.store(value, Ordering::Relaxed);
        if value {
            self.reset.store(true, Ordering::Relaxed);
        }
    }

    /// Сбросить распознавание (после того как Капри сама говорила).
    pub fn request_reset(&self) {
        self.reset.store(true, Ordering::Relaxed);
    }
}

struct Worker {
    engine: Vosk,
    on_partial: TextCallback,
    on_final: FinalCallback,
    on_error: ErrorCallback,
    muted: Arc<AtomicBool>,
    reset: Arc<AtomicBool>,
    level: Arc<AtomicU32>,
    device: Arc<Mutex<Option<String>>>,
    gain: Arc<Mutex<f32>>,
    sensitivity: Arc<AtomicI32>,
    active: Arc<Mutex<Option<String>>>,
}

enum Outcome {
    /// Временный сбой (устройство отвалилось) — пробуем ещё раз.
    Retry,
    /// Дальше слушать нечем.
    Fatal(String),
}

impl Worker {
    fn run(self, switch_rx: Receiver<()>) {
        let mut worked = false;
        loop {
            while switch_rx.try_recv().is_ok() {}
            let device_name = self.device.lock().clone();
            match self.listen_once(device_name.as_deref(), &switch_rx) {
                Ok(()) => worked = true,
                Err(Outcome::Fatal(message)) => {
                    (self.on_error)(&message);
                    return;
                }
                Err(Outcome::Retry) => {
                    if !worked {
                        (self.on_error)("no_mic");
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(3));
                }
            }
        }
    }

    fn listen_once(&self, device_name: Option<&str>, switch_rx: &Receiver<()>) -> Result<(), Outcome> {
        let host = cpal::default_host();
        let Some(device) = pick_device(&host, device_name) else {
            return Err(Outcome::Retry);
        };
        *self.active.lock() = Some(device_name_of(&device));

        let supported = device.default_input_config().map_err(|_| Outcome::Retry)?;
        let sample_format = supported.sample_format();
        let in_rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        let config = supported.config();

        let Some(mut recognizer) = self.engine.new_recognizer(RATE as f32) else {
            return Err(Outcome::Fatal("model_broken".into()));
        };

        let (tx, rx) = mpsc::channel::<Vec<f32>>();
        let stream = match build_stream(&device, &config, sample_format, channels, tx) {
            Ok(stream) => stream,
            Err(e) => {
                log::warn!("Микрофон не открылся: {e}");
                return Err(Outcome::Retry);
            }
        };
        if stream.play().is_err() {
            log::warn!("Не удалось запустить поток с микрофона");
            return Err(Outcome::Retry);
        }

        let mut converter = Converter::new(in_rate, RATE);
        let mut buffer: Vec<i16> = Vec::with_capacity(RATE as usize);
        let mut was_muted = false;
        let mut last_audio = Instant::now();

        loop {
            if switch_rx.try_recv().is_ok() {
                return Ok(());
            }
            let chunk = match rx.recv_timeout(Duration::from_millis(400)) {
                Ok(chunk) => chunk,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Устройство замолчало совсем — пересоздаём поток.
                    if last_audio.elapsed() > Duration::from_secs(6) {
                        return Ok(());
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(Outcome::Retry),
            };
            last_audio = Instant::now();

            let gained = self.apply_gain(&chunk);
            let mono16 = converter.push(&gained);
            if mono16.is_empty() {
                continue;
            }

            if self.muted.load(Ordering::Relaxed) {
                was_muted = true;
                continue;
            }
            if was_muted || self.reset.swap(false, Ordering::Relaxed) {
                was_muted = false;
                recognizer.reset();
                buffer.clear();
            }

            buffer.extend_from_slice(&mono16);
            if recognizer.accept(&buffer) {
                let text = recognizer.result("text");
                let audio = std::mem::take(&mut buffer);
                if !text.is_empty() {
                    (self.on_final)(&text, &audio);
                }
            } else {
                let partial = recognizer.result("partial");
                if !partial.is_empty() {
                    (self.on_partial)(&partial);
                }
                // Не копим больше 25 секунд звука.
                let limit = RATE as usize * 25;
                if buffer.len() > limit {
                    buffer.drain(..limit / 5);
                }
            }
        }
    }

    fn apply_gain(&self, samples: &[f32]) -> Vec<f32> {
        let gain = *self.gain.lock();
        let scaled: Vec<f32> = if (gain - 1.0).abs() > 0.01 {
            samples.iter().map(|s| (s * gain).clamp(-1.0, 1.0)).collect()
        } else {
            samples.to_vec()
        };
        // Индикатор уровня: RMS, как в Python-версии.
        let rms = if scaled.is_empty() {
            0.0
        } else {
            (scaled.iter().map(|s| s * s).sum::<f32>() / scaled.len() as f32).sqrt()
        };
        let level = (rms * LEVEL_SCALE).min(1.0);
        self.level.store(level.to_bits(), Ordering::Relaxed);

        // Шумовой порог: тихое не отдаём распознавателю.
        let gate = (100 - self.sensitivity.load(Ordering::Relaxed)) as f32 / 100.0 * GATE_MAX;
        if level < gate {
            return vec![0.0; scaled.len()];
        }
        scaled
    }
}

fn read_json(ptr: *const std::os::raw::c_char, key: &str) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let text = unsafe { std::ffi::CStr::from_ptr(ptr) }.to_string_lossy().to_string();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return String::new();
    };
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    channels: usize,
    tx: Sender<Vec<f32>>,
) -> Result<cpal::Stream, cpal::Error> {
    // Ошибку только пишем в лог: поток-слушатель сам заметит тишину
    // по таймауту и пересоздаст устройство.
    let on_error = move |e: cpal::Error| {
        log::warn!("Поток микрофона: {e}");
    };
    macro_rules! stream_for {
        ($sample:ty) => {
            device.build_input_stream(
                config.clone(),
                move |data: &[$sample], _: &cpal::InputCallbackInfo| {
                    let _ = tx.send(to_mono(data, channels));
                },
                on_error,
                None,
            )
        };
    }
    match format {
        cpal::SampleFormat::I16 => stream_for!(i16),
        cpal::SampleFormat::F32 => stream_for!(f32),
        cpal::SampleFormat::U16 => stream_for!(u16),
        other => Err(cpal::Error::with_message(
            cpal::ErrorKind::UnsupportedConfig,
            format!("формат микрофона {other:?} не поддержан"),
        )),
    }
}

/// Свести каналы в моно и привести к `f32` в диапазоне −1…1.
fn to_mono<T>(data: &[T], channels: usize) -> Vec<f32>
where
    T: Sample,
    f32: cpal::FromSample<T>,
{
    let channels = channels.max(1);
    data.chunks(channels)
        .map(|frame| {
            let sum: f32 = frame.iter().map(|s| f32::from_sample(*s)).sum();            sum / channels as f32
        })
        .collect()
}

/// Потоковая передискретизация в 16 кГц.
struct Converter {
    from: u32,
    to: u32,
    step: f64,
    phase: f64,
    prev: f32,
    /// Накопитель для целочисленного дециматора.
    acc: f32,
    acc_count: u32,
    /// Кратное соотношение частот (48000 → 16000).
    integer_ratio: Option<u32>,
}

impl Converter {
    fn new(from: u32, to: u32) -> Self {
        let ratio = if to > 0 && from % to == 0 && from != to { Some(from / to) } else { None };
        Self {
            from,
            to,
            step: from as f64 / to.max(1) as f64,
            phase: 0.0,
            prev: 0.0,
            acc: 0.0,
            acc_count: 0,
            integer_ratio: ratio,
        }
    }

    fn push(&mut self, input: &[f32]) -> Vec<i16> {
        let to_i16 = |value: f32| (value * 32767.0).clamp(-32768.0, 32767.0) as i16;
        if self.from == self.to {
            return input.iter().map(|s| to_i16(*s)).collect();
        }

        if let Some(ratio) = self.integer_ratio {
            // Кратные частоты: усредняем по ratio сэмплов — простой ФНЧ,
            // иначе в 16 кГц полезет алиасинг.
            let mut out = Vec::with_capacity(input.len() / ratio as usize + 2);
            for sample in input {
                self.acc += *sample;
                self.acc_count += 1;
                if self.acc_count == ratio {
                    out.push(to_i16(self.acc / ratio as f32));
                    self.acc = 0.0;
                    self.acc_count = 0;
                }
            }
            return out;
        }

        // Некратные частоты: линейная интерполяция.
        let mut out = Vec::with_capacity((input.len() as f64 / self.step) as usize + 2);
        for sample in input {
            self.phase += 1.0;
            while self.phase >= self.step {
                let frac = (((self.phase - self.step) / self.step) as f32).clamp(0.0, 1.0);
                let value = self.prev + (*sample - self.prev) * frac;
                out.push(to_i16(value));
                self.phase -= self.step;
            }
            self.prev = *sample;
        }
        out
    }
}

fn pick_device(host: &cpal::Host, name: Option<&str>) -> Option<cpal::Device> {
    let Some(wanted) = name.map(|n| n.trim().to_lowercase()).filter(|n| !n.is_empty()) else {
        return host.default_input_device();
    };
    if let Ok(devices) = host.input_devices() {
        let all: Vec<cpal::Device> = devices.collect();
        // Точное совпадение.
        for device in &all {
            if device_name_of(device).to_lowercase() == wanted {
                return Some(device.clone());
            }
        }
        // Мягкое совпадение: первые 20 символов, как в Python-версии.
        let short: String = wanted.chars().take(20).collect();
        for device in &all {
            let current = device_name_of(device).to_lowercase();
            let current_short: String = current.chars().take(20).collect();
            if (!short.is_empty() && current.contains(&short)) || current_short.contains(&wanted) {
                return Some(device.clone());
            }
        }
    }
    // Запрошенного микрофона нет — берём обычный.
    host.default_input_device()
}

fn device_name_of(device: &cpal::Device) -> String {
    match device.description() {
        Ok(description) => description.name().to_string(),
        Err(_) => "неизвестно".to_string(),
    }
}

/// Список микрофонов для меню и настроек.
pub fn input_devices() -> Vec<String> {
    let host = cpal::default_host();
    let mut out: Vec<String> = Vec::new();
    if let Ok(devices) = host.input_devices() {
        for device in devices {
            let name = device_name_of(&device);
            if name.trim().is_empty() || name == "неизвестно" {
                continue;
            }
            let low = name.to_lowercase();
            // Служебные дубли Windows не показываем.
            if low.contains("sound mapper")
                || low.contains("переназначение звуковых")
                || low.contains("первичный драйвер")
            {
                continue;
            }
            if !out.iter().any(|existing| existing == &name) {
                out.push(name);
            }
        }
    }
    out
}

/// Имя микрофона по умолчанию.
pub fn default_input_name() -> String {
    let host = cpal::default_host();
    host.default_input_device()
        .map(|d| device_name_of(&d))
        .unwrap_or_else(|| "неизвестно".to_string())
}

/// Модель Vosk на месте и полная?
pub fn model_ready(cfg: &Config, base_dir: &Path) -> bool {
    model_complete(&base_dir.join(&cfg.vosk_model))
}

/// В модели есть `am/final.mdl` — значит, скачана целиком.
pub fn model_complete(dir: &Path) -> bool {
    dir.join("am").join("final.mdl").is_file()
}

/// Vosk не умеет открывать пути с русскими буквами — копируем модель
/// в ASCII-путь, как это делала Python-версия.
pub fn ascii_model_path(dir: &Path) -> Option<PathBuf> {
    if dir.to_string_lossy().is_ascii() {
        return Some(dir.to_path_buf());
    }
    if !dir.is_dir() {
        return None;
    }
    let name = dir.file_name()?.to_string_lossy().to_string();
    let bases = [
        std::env::var("LOCALAPPDATA").unwrap_or_default(),
        std::env::var("PROGRAMDATA").unwrap_or_else(|_| "C:\\ProgramData".into()),
        std::env::var("PUBLIC").unwrap_or_else(|_| "C:\\Users\\Public".into()),
        std::env::temp_dir().to_string_lossy().to_string(),
    ];
    for base in bases {
        if base.is_empty() {
            continue;
        }
        let target = PathBuf::from(&base).join("Capri").join(&name);
        if !target.to_string_lossy().is_ascii() {
            continue;
        }
        if model_complete(&target) {
            return Some(target);
        }
        if copy_dir(dir, &target).is_ok() && model_complete(&target) {
            return Some(target);
        }
    }
    log::warn!(
        "Модель лежит по пути с русскими буквами, а Vosk такое не открывает: {}",
        dir.display()
    );
    None
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Загруженная `libvosk.dll` и её функции.
///
/// Все небезопасные вызовы спрятаны внутри методов: снаружи это обычные
/// безопасные функции с русскими ошибками.
struct Vosk {
    library: libloading::Library,
    model: *mut std::ffi::c_void,
    recognizer_new: unsafe extern "C" fn(*const std::ffi::c_void, f32) -> *mut std::ffi::c_void,
    recognizer_free: unsafe extern "C" fn(*mut std::ffi::c_void),
    recognizer_reset: unsafe extern "C" fn(*mut std::ffi::c_void),
    recognizer_accept: unsafe extern "C" fn(*mut std::ffi::c_void, *const i8, i32) -> i32,
    recognizer_result: unsafe extern "C" fn(*mut std::ffi::c_void) -> *const std::os::raw::c_char,
    recognizer_partial: unsafe extern "C" fn(*mut std::ffi::c_void) -> *const std::os::raw::c_char,
    model_free: unsafe extern "C" fn(*mut std::ffi::c_void),
}

// Библиотека живёт столько же, сколько указатели, — передаём её между потоками.
unsafe impl Send for Vosk {}

/// Распознаватель, который сам освобождается.
struct Recognizer {
    handle: *mut std::ffi::c_void,
    free: unsafe extern "C" fn(*mut std::ffi::c_void),
    reset_fn: unsafe extern "C" fn(*mut std::ffi::c_void),
    accept_fn: unsafe extern "C" fn(*mut std::ffi::c_void, *const i8, i32) -> i32,
    result_fn: unsafe extern "C" fn(*mut std::ffi::c_void) -> *const std::os::raw::c_char,
    partial_fn: unsafe extern "C" fn(*mut std::ffi::c_void) -> *const std::os::raw::c_char,
}

impl Recognizer {
    fn reset(&mut self) {
        unsafe { (self.reset_fn)(self.handle) };
    }

    /// Отдать звук распознавателю. `true` — фраза закончилась, есть результат.
    fn accept(&mut self, samples: &[i16]) -> bool {
        if samples.is_empty() {
            return false;
        }
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let code = unsafe {
            (self.accept_fn)(self.handle, bytes.as_ptr().cast::<i8>(), bytes.len() as i32)
        };
        code > 0
    }

    /// Поле из JSON-ответа (`text` или `partial`).
    fn result(&self, key: &str) -> String {
        let ptr = unsafe {
            if key == "text" {
                (self.result_fn)(self.handle)
            } else {
                (self.partial_fn)(self.handle)
            }
        };
        read_json(ptr, key)
    }
}

impl Drop for Recognizer {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { (self.free)(self.handle) };
        }
    }
}

impl Vosk {
    fn load() -> Result<Self, String> {
        let library = load_library()?;
        unsafe {
            let set_log: libloading::Symbol<unsafe extern "C" fn(i32)> =
                library.get(b"vosk_set_log_level").map_err(|e| e.to_string())?;
            set_log(-1);

            let recognizer_new = *library
                .get::<unsafe extern "C" fn(*const std::ffi::c_void, f32) -> *mut std::ffi::c_void>(
                    b"vosk_recognizer_new",
                )
                .map_err(|e| e.to_string())?;
            let recognizer_free = *library
                .get::<unsafe extern "C" fn(*mut std::ffi::c_void)>(b"vosk_recognizer_free")
                .map_err(|e| e.to_string())?;
            let recognizer_reset = *library
                .get::<unsafe extern "C" fn(*mut std::ffi::c_void)>(b"vosk_recognizer_reset")
                .map_err(|e| e.to_string())?;
            let recognizer_accept = *library
                .get::<unsafe extern "C" fn(*mut std::ffi::c_void, *const i8, i32) -> i32>(
                    b"vosk_recognizer_accept_waveform",
                )
                .map_err(|e| e.to_string())?;
            let recognizer_result = *library
                .get::<unsafe extern "C" fn(*mut std::ffi::c_void) -> *const std::os::raw::c_char>(
                    b"vosk_recognizer_result",
                )
                .map_err(|e| e.to_string())?;
            let recognizer_partial = *library
                .get::<unsafe extern "C" fn(*mut std::ffi::c_void) -> *const std::os::raw::c_char>(
                    b"vosk_recognizer_partial_result",
                )
                .map_err(|e| e.to_string())?;
            let model_free = *library
                .get::<unsafe extern "C" fn(*mut std::ffi::c_void)>(b"vosk_model_free")
                .map_err(|e| e.to_string())?;

            Ok(Self {
                library,
                model: std::ptr::null_mut(),
                recognizer_new,
                recognizer_free,
                recognizer_reset,
                recognizer_accept,
                recognizer_result,
                recognizer_partial,
                model_free,
            })
        }
    }

    fn open_model(&mut self, path: &Path) -> Result<(), String> {
        let ascii = path.to_string_lossy().replace('/', "\\");
        let c_path =
            std::ffi::CString::new(ascii).map_err(|_| "путь к модели с нулевым байтом".to_string())?;
        let model_new = unsafe {
            *self
                .library
                .get::<unsafe extern "C" fn(*const std::os::raw::c_char) -> *mut std::ffi::c_void>(
                    b"vosk_model_new",
                )
                .map_err(|e| e.to_string())?
        };
        let model = unsafe { model_new(c_path.as_ptr()) };
        if model.is_null() {
            return Err("модель не открылась".into());
        }
        self.model = model;
        Ok(())
    }

    /// Новый распознаватель под частоту `rate`.
    fn new_recognizer(&self, rate: f32) -> Option<Recognizer> {
        let handle = unsafe { (self.recognizer_new)(self.model, rate) };
        if handle.is_null() {
            return None;
        }
        let mut recognizer = Recognizer {
            handle,
            free: self.recognizer_free,
            reset_fn: self.recognizer_reset,
            accept_fn: self.recognizer_accept,
            result_fn: self.recognizer_result,
            partial_fn: self.recognizer_partial,
        };
        recognizer.reset();
        Some(recognizer)
    }
}

impl Drop for Vosk {
    fn drop(&mut self) {
        if !self.model.is_null() {
            unsafe { (self.model_free)(self.model) };
        }
    }
}

fn load_library() -> Result<libloading::Library, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("libvosk.dll"));
            candidates.push(dir.join("vosk").join("libvosk.dll"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("libvosk.dll"));
    }
    candidates.push(PathBuf::from("libvosk.dll"));

    let mut last_error = String::from("libvosk.dll не найдена");
    for candidate in candidates {
        let plain = candidate == PathBuf::from("libvosk.dll");
        if !plain && !candidate.is_file() {
            continue;
        }
        match unsafe { libloading::Library::new(&candidate) } {
            Ok(library) => return Ok(library),
            Err(e) => last_error = format!("{}: {e}", candidate.display()),
        }
    }
    Err(last_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_complete_model() {
        let dir = std::env::temp_dir().join("capri-test-model");
        let am = dir.join("am");
        std::fs::create_dir_all(&am).expect("каталог");
        std::fs::write(am.join("final.mdl"), b"x").expect("файл");
        assert!(model_complete(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn converts_multi_channel_to_mono() {
        let mono = to_mono(&[0.5f32, 0.5, -1.0, -1.0], 2);
        assert_eq!(mono.len(), 2);
        assert!((mono[0] - 0.5).abs() < 0.001);
        assert!((mono[1] + 1.0).abs() < 0.001);
    }

    #[test]
    fn downsampler_decimates_by_three() {
        let mut converter = Converter::new(48_000, 16_000);
        let out = converter.push(&vec![0.5f32; 4800]);
        assert_eq!(out.len(), 1600);
    }

    #[test]
    fn resampler_handles_odd_ratio() {
        let mut converter = Converter::new(44_100, 16_000);
        let out = converter.push(&vec![0.25f32; 4410]);
        assert!((out.len() as i32 - 1600).abs() <= 3, "получилось {}", out.len());
    }

    #[test]
    fn keeps_rate_when_equal() {
        let mut converter = Converter::new(16_000, 16_000);
        let out = converter.push(&vec![0.1f32; 100]);
        assert_eq!(out.len(), 100);
    }
}

