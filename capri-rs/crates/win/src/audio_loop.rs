//! Визуализатор звука для медиа-виджета.
//!
//! Захватываем то, что реально играет в наушниках/колонках: WASAPI loopback
//! с устройства вывода по умолчанию. Из потока берём окно в 1024 сэмпла,
//! считаем амплитуду в 16 полосах алгоритмом Гёрцеля (без FFT и без новых
//! зависимостей) и публикуем сглаженные значения раз в ~80 мс.
//!
//! Публичный API:
//! - [`start`] — один раз поднять фоновый поток;
//! - [`bands`] — 16 полос 0.0…1.0, пустой вектор, если визуализатор недоступен;
//! - [`latest`] — те же полосы, но без обращения к аудио (можно звать каждый кадр).
//!
//! Ошибки COM/WASAPI не паникуют: поток пишет одну строку в `log` и завершается,
//! а виджет получает пустой вектор — лучше пустой эквалайзер, чем упавшее приложение.

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Сколько полос эквалайзера.
const BANDS: usize = 16;
/// Размер анализируемого окна (сэмплов моно).
const SAMPLE_COUNT: usize = 1024;
/// Храним не больше двух окон — больше для анализа не нужно.
const KEEP_SAMPLES: usize = SAMPLE_COUNT * 2;
/// Период публикации новых значений.
const EMIT_EVERY: Duration = Duration::from_millis(80);
/// Пауза между опросами кольцевого буфера loopback.
const POLL_EVERY: Duration = Duration::from_millis(10);
/// За один опрос разбираем не больше стольких пакетов, чтобы не залипнуть.
const MAX_PACKETS_PER_TICK: usize = 64;
/// Нижняя граница сетки частот, Гц.
const LOW_HZ: f32 = 60.0;
/// Верхняя граница сетки частот, Гц.
const HIGH_HZ: f32 = 12_000.0;
/// Усиление перед логарифмом: тихая музыка тоже должна быть видна.
const BAND_GAIN: f32 = 6.0;
/// Ниже этой средней энергии считаем, что звука нет, и отдаём нули.
const SILENCE_ENERGY: f32 = 1e-9;
/// Веса сглаживания по времени: новая = 0.6 * старая + 0.4 * свежая.
const OLD_WEIGHT: f32 = 0.6;
const NEW_WEIGHT: f32 = 0.4;

/// Последние посчитанные полосы. Пусто — ещё не считали или визуализатор умер.
static LATEST: Lazy<Mutex<Vec<f32>>> = Lazy::new(|| Mutex::new(Vec::new()));
/// Поток уже запускали: повторный [`start`] ничего не делает.
static STARTED: AtomicBool = AtomicBool::new(false);
/// Захват действительно работает (поток жив и поток аудио открыт).
static ALIVE: AtomicBool = AtomicBool::new(false);

// ------------------------------------------------------------ чистая математика

/// Амплитуда синусоиды на частоте `freq` в окне `samples` (алгоритм Гёрцеля).
///
/// Для синусоиды амплитуды `A` на этой же частоте результат ≈ `A`.
fn goertzel(samples: &[f32], freq: f32, sample_rate: f32) -> f32 {
    let n = samples.len();
    if n < 2 || !(freq > 0.0) || !(sample_rate > 0.0) {
        return 0.0;
    }
    let normalized = freq / sample_rate;
    if !(normalized > 0.0 && normalized < 0.5) {
        return 0.0;
    }
    let omega = 2.0 * std::f32::consts::PI * normalized;
    let coeff = 2.0 * omega.cos();
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    for &sample in samples {
        let s0 = sample + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let real = s1 - s2 * omega.cos();
    let imag = s2 * omega.sin();
    let amplitude = (real * real + imag * imag).sqrt() / (n as f32 * 0.5);
    if amplitude.is_finite() && amplitude > 0.0 {
        amplitude
    } else {
        0.0
    }
}

/// Центры полос: логарифмическая сетка от [`LOW_HZ`] до [`HIGH_HZ`].
fn band_centers() -> [f32; BANDS] {
    let mut centers = [0.0f32; BANDS];
    let ratio = HIGH_HZ / LOW_HZ;
    for (index, center) in centers.iter_mut().enumerate() {
        // Середина i-й полосы в логарифмическом масштабе.
        let t = (index as f32 + 0.5) / BANDS as f32;
        *center = LOW_HZ * ratio.powf(t);
    }
    centers
}

/// Нормализация амплитуды: логарифм, чтобы тихие полосы не пропадали, затем 0.0…1.0.
fn normalize(magnitude: f32, gain: f32) -> f32 {
    if !magnitude.is_finite() || magnitude <= 0.0 {
        return 0.0;
    }
    (1.0 + magnitude * gain).ln().clamp(0.0, 1.0)
}

/// Свежие полосы из окна моно-сэмплов. Тишина — ровно [`BANDS`] нулей.
fn compute_bands(samples: &[f32], sample_rate: f32) -> Vec<f32> {
    let mut out = vec![0.0f32; BANDS];
    if samples.len() < 2 || !(sample_rate > 0.0) {
        return out;
    }
    let energy = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
    if !energy.is_finite() || energy < SILENCE_ENERGY {
        return out;
    }
    let nyquist = sample_rate * 0.5;
    let centers = band_centers();
    for (value, freq) in out.iter_mut().zip(centers.iter()) {
        if !(*freq > 0.0) || *freq >= nyquist {
            continue;
        }
        *value = normalize(goertzel(samples, *freq, sample_rate), BAND_GAIN);
    }
    out
}

/// Сглаживание по времени: `0.6 * старое + 0.4 * свежее`, всегда в 0.0…1.0.
///
/// `prev` обновляется на месте, возвращаются новые значения.
fn smooth(prev: &mut Vec<f32>, fresh: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(fresh.len());
    for (index, value) in fresh.iter().enumerate() {
        let old = prev.get(index).copied().unwrap_or(*value);
        out.push((OLD_WEIGHT * old + NEW_WEIGHT * value).clamp(0.0, 1.0));
    }
    *prev = out.clone();
    out
}

// ------------------------------------------------------------------- публичный API

/// Полосы эквалайзера: 16 значений 0.0…1.0 (низкие частоты → высокие).
/// Пустой вектор — звука нет / визуализатор недоступен.
pub fn bands() -> Vec<f32> {
    if !ALIVE.load(Ordering::SeqCst) {
        return Vec::new();
    }
    latest()
}

/// Запустить фоновый поток, который раз в ~80 мс обновляет полосы.
/// Повторный вызов — no-op.
pub fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    #[cfg(windows)]
    {
        let spawned = std::thread::Builder::new()
            .name("capri-audio-loop".into())
            .spawn(|| imp::run());
        if let Err(err) = spawned {
            log::warn!("визуализатор звука: поток не запустился: {err}");
            // Даём шанс попробовать снова: причина может быть временной.
            STARTED.store(false, Ordering::SeqCst);
        }
    }
    #[cfg(not(windows))]
    {
        log::debug!("визуализатор звука: захват loopback есть только на Windows");
    }
}

/// Последние посчитанные полосы (быстро, без обращения к аудио).
pub fn latest() -> Vec<f32> {
    LATEST.lock().clone()
}

// ------------------------------------------------------------------- Windows: capture

#[cfg(windows)]
mod imp {
    use super::{smooth, compute_bands, ALIVE, BANDS, EMIT_EVERY, LATEST, MAX_PACKETS_PER_TICK, POLL_EVERY, SAMPLE_COUNT, KEEP_SAMPLES};
    use std::sync::atomic::Ordering;
    use std::time::Instant;
    use windows::core::GUID;
    use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
        MMDeviceEnumerator, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, AUDCLNT_BUFFERFLAGS_SILENT,
        AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
        COINIT_MULTITHREADED,
    };

    /// 100 мс в единицах по 100 нс — длина буфера loopback.
    const BUFFER_DURATION_HNS: i64 = 1_000_000;

    // Значения из mmreg.h/ksmedia.h: держим их здесь, чтобы не тянуть лишние фичи windows.
    const WAVE_FORMAT_PCM_TAG: u16 = 1;
    const WAVE_FORMAT_IEEE_FLOAT_TAG: u16 = 3;
    const WAVE_FORMAT_EXTENSIBLE_TAG: u16 = 0xFFFE;
    const SUBFORMAT_PCM: GUID = GUID::from_u128(0x0000_0001_0000_0010_8000_00aa00389b71);
    const SUBFORMAT_IEEE_FLOAT: GUID = GUID::from_u128(0x0000_0003_0000_0010_8000_00aa00389b71);

    /// Как читать сэмплы микшера.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum SampleKind {
        F32,
        I16,
    }

    impl SampleKind {
        fn bytes(self) -> usize {
            match self {
                SampleKind::F32 => 4,
                SampleKind::I16 => 2,
            }
        }
    }

    /// Разобранный формат микшера — всё, что нужно для чтения буфера.
    struct MixFormat {
        channels: usize,
        sample_rate: u32,
        block_align: usize,
        kind: SampleKind,
    }

    /// Инициализация COM для своего потока: `CoUninitialize` вызывается в Drop.
    struct ComGuard {
        initialized: bool,
    }

    impl ComGuard {
        fn new() -> Option<Self> {
            // S_FALSE (уже инициализирован) — тоже успех, CoUninitialize нужен и там.
            let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if hr.is_ok() {
                Some(Self { initialized: true })
            } else {
                let code = hr.0;
                if code == RPC_E_CHANGED_MODE.0 {
                    log::debug!("визуализатор звука: COM уже занят в режиме STA");
                } else {
                    log::debug!("визуализатор звука: CoInitializeEx вернул 0x{code:08X}");
                }
                None
            }
        }
    }

    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.initialized {
                unsafe { CoUninitialize() };
            }
        }
    }

    /// Фоновый поток визуализатора: одна попытка открыть loopback, затем цикл.
    pub(super) fn run() {
        let _com = match ComGuard::new() {
            Some(guard) => guard,
            None => return,
        };
        match capture() {
            Ok(()) => log::debug!("визуализатор звука: захват завершён"),
            Err(err) => log::debug!("визуализатор звука недоступен: {err}"),
        }
        ALIVE.store(false, Ordering::SeqCst);
        // Не отдаём виджету устаревшую картинку, если поток умер.
        LATEST.lock().clear();
    }

    /// Открыть loopback и крутить цикл чтения до первой фатальной ошибки.
    fn capture() -> Result<(), String> {
        let enumerator: IMMDeviceEnumerator = unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        }
        .map_err(|err| format!("перечислитель устройств: {err}"))?;

        let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
            .map_err(|err| format!("устройство вывода по умолчанию: {err}"))?;

        let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }
            .map_err(|err| format!("аудиоклиент: {err}"))?;

        let mix_ptr = unsafe { client.GetMixFormat() }
            .map_err(|err| format!("формат микшера: {err}"))?;
        let parsed = describe(mix_ptr);
        let format = match parsed {
            Ok(format) => format,
            Err(err) => {
                unsafe { CoTaskMemFree(Some(mix_ptr as *const std::ffi::c_void)) };
                return Err(err);
            }
        };

        // В общем режиме loopback разрешён только с флагом LOOPBACK.
        let init = unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                BUFFER_DURATION_HNS,
                0,
                mix_ptr,
                None,
            )
        };
        // Формат скопирован внутрь клиента — память микшера можно освободить.
        unsafe { CoTaskMemFree(Some(mix_ptr as *const std::ffi::c_void)) };
        init.map_err(|err| format!("loopback не открылся: {err}"))?;

        let capture_client: IAudioCaptureClient = unsafe { client.GetService() }
            .map_err(|err| format!("клиент захвата: {err}"))?;

        unsafe { client.Start() }.map_err(|err| format!("поток не запустился: {err}"))?;
        ALIVE.store(true, Ordering::SeqCst);

        let result = read_loop(&capture_client, &format);
        let _ = unsafe { client.Stop() };
        result
    }

    /// Разобрать `WAVEFORMATEX` из микшера. Указатель живёт до `CoTaskMemFree`.
    fn describe(format: *const WAVEFORMATEX) -> Result<MixFormat, String> {
        if format.is_null() {
            return Err("микшер вернул пустой формат".into());
        }
        // WAVEFORMATEX — packed-структура, читаем копией без ссылок на поля.
        let base = unsafe { std::ptr::read_unaligned(format) };
        let tag = base.wFormatTag;
        let channels = base.nChannels as usize;
        let sample_rate = base.nSamplesPerSec;
        let block_align = base.nBlockAlign as usize;
        let bits = base.wBitsPerSample;

        if channels == 0 || channels > 32 {
            return Err(format!("странное число каналов: {channels}"));
        }
        if sample_rate == 0 {
            return Err("микшер вернул нулевую частоту".into());
        }
        if block_align < channels {
            return Err(format!("битый block_align: {block_align}"));
        }

        let mut kind = None;
        match tag {
            WAVE_FORMAT_IEEE_FLOAT_TAG => {
                if bits == 32 {
                    kind = Some(SampleKind::F32);
                }
            }
            WAVE_FORMAT_PCM_TAG => {
                if bits == 16 {
                    kind = Some(SampleKind::I16);
                }
            }
            WAVE_FORMAT_EXTENSIBLE_TAG => {
                if base.cbSize >= 22 {
                    let ext = unsafe { std::ptr::read_unaligned(format as *const WAVEFORMATEXTENSIBLE) };
                    let sub: GUID = ext.SubFormat;
                    if sub == SUBFORMAT_IEEE_FLOAT && bits == 32 {
                        kind = Some(SampleKind::F32);
                    } else if sub == SUBFORMAT_PCM && bits == 16 {
                        kind = Some(SampleKind::I16);
                    }
                }
            }
            _ => {}
        }

        let kind = kind.ok_or_else(|| {
            format!("незнакомый формат микшера: тег {tag}, {bits} бит")
        })?;

        let sample_bytes = block_align / channels;
        if sample_bytes != kind.bytes() {
            return Err(format!(
                "размер сэмпла {sample_bytes} не совпадает с форматом ({} байт)",
                kind.bytes()
            ));
        }

        Ok(MixFormat {
            channels,
            sample_rate,
            block_align,
            kind,
        })
    }

    /// Основной цикл: выгребаем пакеты, раз в ~80 мс публикуем полосы.
    fn read_loop(capture: &IAudioCaptureClient, format: &MixFormat) -> Result<(), String> {
        let mut mono: Vec<f32> = Vec::with_capacity(KEEP_SAMPLES);
        let mut smoothed: Vec<f32> = Vec::new();
        let mut last_emit = Instant::now();

        loop {
            for _ in 0..MAX_PACKETS_PER_TICK {
                let packet = unsafe { capture.GetNextPacketSize() }
                    .map_err(|err| format!("размер пакета: {err}"))?;
                if packet == 0 {
                    break;
                }

                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames: u32 = 0;
                let mut flags: u32 = 0;
                unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None) }
                    .map_err(|err| format!("буфер пакета: {err}"))?;

                if frames > 0 {
                    let silent =
                        flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null();
                    if silent {
                        // Тишина тоже двигает время: дописываем нули.
                        mono.resize(mono.len() + frames as usize, 0.0);
                    } else {
                        let bytes = (frames as usize).saturating_mul(format.block_align);
                        let raw = unsafe { std::slice::from_raw_parts(data, bytes) };
                        decode_into(raw, format, &mut mono);
                    }
                }

                unsafe { capture.ReleaseBuffer(frames) }
                    .map_err(|err| format!("буфер не освобождён: {err}"))?;
            }

            if mono.len() > KEEP_SAMPLES {
                let extra = mono.len() - KEEP_SAMPLES;
                mono.drain(..extra);
            }

            if last_emit.elapsed() >= EMIT_EVERY {
                let fresh = if mono.len() >= SAMPLE_COUNT {
                    let window = &mono[mono.len() - SAMPLE_COUNT..];
                    compute_bands(window, format.sample_rate as f32)
                } else {
                    // Данных ещё мало — это тишина.
                    vec![0.0f32; BANDS]
                };
                let values = smooth(&mut smoothed, &fresh);
                *LATEST.lock() = values;
                last_emit = Instant::now();
            }

            std::thread::sleep(POLL_EVERY);
        }
    }

    /// Разложить блок PCM в моно-сэмплы: каналы усредняются.
    fn decode_into(raw: &[u8], format: &MixFormat, out: &mut Vec<f32>) {
        let frame_bytes = format.block_align;
        if frame_bytes == 0 || format.channels == 0 {
            return;
        }
        let sample_bytes = frame_bytes / format.channels;
        let frames = raw.len() / frame_bytes;
        out.reserve(frames);
        for frame in 0..frames {
            let base = frame * frame_bytes;
            let mut sum = 0.0f32;
            for channel in 0..format.channels {
                let offset = base + channel * sample_bytes;
                let value = match format.kind {
                    SampleKind::F32 => read_f32(raw, offset).unwrap_or(0.0),
                    SampleKind::I16 => read_i16(raw, offset).unwrap_or(0.0) / 32_768.0,
                };
                sum += value;
            }
            out.push((sum / format.channels as f32).clamp(-1.0, 1.0));
        }
    }

    fn read_f32(raw: &[u8], offset: usize) -> Option<f32> {
        let slice = raw.get(offset..offset + 4)?;
        let bytes: [u8; 4] = slice.try_into().ok()?;
        Some(f32::from_le_bytes(bytes))
    }

    fn read_i16(raw: &[u8], offset: usize) -> Option<f32> {
        let slice = raw.get(offset..offset + 2)?;
        let bytes: [u8; 2] = slice.try_into().ok()?;
        Some(i16::from_le_bytes(bytes) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: f32, count: usize) -> Vec<f32> {
        (0..count)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate).sin())
            .collect()
    }

    #[test]
    fn goertzel_finds_tone() {
        let rate = 48_000.0;
        let samples = sine(1_000.0, rate, SAMPLE_COUNT);
        let at_tone = goertzel(&samples, 1_000.0, rate);
        let off_tone = goertzel(&samples, 400.0, rate);
        assert!((at_tone - 1.0).abs() < 0.05, "амплитуда тона: {at_tone}");
        assert!(off_tone < at_tone * 0.2, "вне тона слишком много: {off_tone}");
        // Пустое окно и частота за Найквистом не должны давать энергии.
        assert_eq!(goertzel(&[], 1_000.0, rate), 0.0);
        assert_eq!(goertzel(&samples, 30_000.0, rate), 0.0);
    }

    #[test]
    fn normalize_silence_gives_zeros() {
        assert_eq!(normalize(0.0, BAND_GAIN), 0.0);
        assert_eq!(normalize(-1.0, BAND_GAIN), 0.0);
        assert!(normalize(0.05, BAND_GAIN) > 0.0);
        assert_eq!(normalize(1e6, BAND_GAIN), 1.0);
        assert_eq!(normalize(f32::INFINITY, BAND_GAIN), 0.0);
        assert!(normalize(0.3, BAND_GAIN) <= 1.0);

        let silence = compute_bands(&vec![0.0f32; SAMPLE_COUNT], 44_100.0);
        assert_eq!(silence.len(), BANDS);
        assert!(silence.iter().all(|value| *value == 0.0));

        // Логарифмическая сетка полос: ровно 16 значений, по возрастанию.
        let centers = band_centers();
        assert!(centers[0] >= LOW_HZ && centers[BANDS - 1] <= HIGH_HZ);
        assert!(centers.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn smoothing_uses_weights_and_stays_in_range() {
        let mut prev: Vec<f32> = Vec::new();
        let fresh = vec![1.0f32; BANDS];

        // Первый кадр без истории берётся как есть.
        let first = smooth(&mut prev, &fresh);
        assert_eq!(first.len(), BANDS);
        assert!((first[0] - 1.0).abs() < 1e-6);

        // Неподвижный сигнал сглаживание не двигает: 0.6*1 + 0.4*1 = 1.0.
        // Раньше здесь ждали 0.8 — арифметика весов такого дать не может.
        let same = smooth(&mut prev, &fresh);
        assert!(
            (same[0] - 1.0).abs() < 1e-6,
            "неподвижный сигнал должен остаться единицей, а получилось {}",
            same[0]
        );

        // Свежие нули: остаётся только доля старого значения — 0.6.
        let decay = smooth(&mut prev, &vec![0.0f32; BANDS]);
        assert!(
            (decay[0] - OLD_WEIGHT).abs() < 1e-6,
            "от затухания должна остаться норма OLD_WEIGHT, а получилось {}",
            decay[0]
        );
        assert!(decay.iter().all(|value| (0.0..=1.0).contains(value)));

        // Резкий рост от 0.6: 0.6*0.6 + 0.4*1.0 = 0.76 — новое берётся с весом 0.4.
        let rise = smooth(&mut prev, &fresh);
        let expected = OLD_WEIGHT * OLD_WEIGHT + NEW_WEIGHT;
        assert!(
            (rise[0] - expected).abs() < 1e-6,
            "рост должен дать {expected}, а получилось {}",
            rise[0]
        );
        assert!(rise.iter().all(|value| (0.0..=1.0).contains(value)));
    }
}
