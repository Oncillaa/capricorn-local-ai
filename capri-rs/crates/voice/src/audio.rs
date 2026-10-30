//! Звук: воспроизведение, «цифровой» эффект голоса и звук глитча.
//!
//! Перенос `voice.py`: `decode`/`trim_silence`/`write_wav`/`fx_process`/
//! `pitch_shift`/`make_glitch_sound`/`play_sound_async`.

use std::io::Cursor;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

/// Частота дискретизации для обработки эффектами (как `FX_SR` в Python).
pub const FX_SR: u32 = 24_000;

/// Текущее воспроизведение: его можно прервать.
static CURRENT: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

/// Громкость и позиция, чтобы UI мог рисовать индикатор.
pub static PLAYING: AtomicBool = AtomicBool::new(false);
static PLAYED_FRAMES: AtomicU64 = AtomicU64::new(0);

/// Открыть звуковое устройство и проиграть готовые сэмплы.
/// Блокирует поток до конца воспроизведения. `true` — прервали.
pub fn play_samples(samples: Vec<f32>, sample_rate: u32, volume: u8) -> bool {
    if samples.is_empty() {
        return false;
    }
    let stop = Arc::new(AtomicBool::new(false));
    *CURRENT.lock() = Some(Arc::clone(&stop));

    let handle = match rodio::DeviceSinkBuilder::open_default_sink() {
        Ok(handle) => handle,
        Err(e) => {
            log::warn!("Нет звукового устройства: {e}");
            *CURRENT.lock() = None;
            return false;
        }
    };
    let player = rodio::Player::connect_new(handle.mixer());
    player.set_volume(volume as f32 / 100.0);

    let gain = if volume == 0 { 0.0 } else { 1.0 };
    let scaled: Vec<f32> = samples.iter().map(|s| s * gain).collect();
    let channels = std::num::NonZeroU16::new(1).unwrap_or(std::num::NonZeroU16::MIN);
    let rate = std::num::NonZeroU32::new(sample_rate).unwrap_or(std::num::NonZeroU32::MIN);
    player.append(rodio::buffer::SamplesBuffer::new(channels, rate, scaled));

    PLAYING.store(true, Ordering::Relaxed);
    PLAYED_FRAMES.store(0, Ordering::Relaxed);
    let mut interrupted = false;
    while !player.empty() {
        if stop.load(Ordering::Relaxed) {
            player.stop();
            interrupted = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    PLAYING.store(false, Ordering::Relaxed);
    *CURRENT.lock() = None;
    interrupted
}

/// Прервать текущую речь.
pub fn stop_playback() {
    if let Some(flag) = CURRENT.lock().as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
}

/// Проиграть файл целиком (MP3/WAV) — просто, без эффектов.
pub fn play_file_sync(path: &Path, volume: u8) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        log::warn!("Не открыла файл для проигрывания: {}", path.display());
        return false;
    };
    let Ok(decoder) = rodio::Decoder::try_from(file) else {
        log::warn!("Не смогла декодировать {}", path.display());
        return false;
    };
    let samples: Vec<f32> = decoder.collect();
    play_samples(samples, FX_SR, volume)
}

/// Проиграть файл в отдельном потоке.
pub fn play_async(path: &Path, volume: u8) {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        play_file_sync(&path, volume);
    });
}

/// Декодировать MP3/WAV из памяти в моно-сэмплы `FX_SR`.
pub fn decode_bytes(bytes: &[u8], sample_rate: u32) -> Result<Vec<f32>, String> {
    use rodio::source::Source;
    let cursor = Cursor::new(bytes.to_vec());
    let decoder = rodio::Decoder::try_from(cursor).map_err(|e| format!("декодер: {e}"))?;
    let channels = decoder.channels().get() as usize;
    let rate = decoder.sample_rate().get();
    let raw: Vec<f32> = decoder.collect();
    // Свести в моно.
    let mono: Vec<f32> = if channels <= 1 {
        raw
    } else {
        raw.chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    };
    if rate == 0 || rate == sample_rate {
        return Ok(mono);
    }
    Ok(resample(&mono, rate, sample_rate))
}

/// Линейная передискретизация.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() || from == 0 {
        return input.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let out_len = (input.len() as f64 * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 / ratio;
        let idx = pos.floor() as usize;
        let frac = (pos - idx as f64) as f32;
        let a = input.get(idx).copied().unwrap_or(0.0);
        let b = input.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

/// Обрезать тишину по краям (как `trim_silence`).
pub fn trim_silence(input: &[f32], sample_rate: u32, threshold: f32) -> Vec<f32> {
    let loud: Vec<usize> = input
        .iter()
        .enumerate()
        .filter(|(_, s)| s.abs() > threshold)
        .map(|(i, _)| i)
        .collect();
    let Some(&first) = loud.first() else {
        return Vec::new();
    };
    let last = loud.last().copied().unwrap_or(first);
    let pad = (sample_rate as f32 * 0.04) as usize;
    let start = first.saturating_sub(pad);
    let end = (last + pad).min(input.len());
    input[start..end].to_vec()
}

/// Сдвиг высоты голоса на полутона.
pub fn pitch_shift(input: &[f32], semitones: f32) -> Vec<f32> {
    if input.is_empty() || semitones.abs() < 0.01 {
        return input.to_vec();
    }
    let factor = 2.0f32.powf(semitones / 12.0);
    let out_len = (input.len() as f32 / factor) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f32 * factor;
        let idx = pos.floor() as usize;
        let frac = pos - idx as f32;
        let a = input.get(idx).copied().unwrap_or(0.0);
        let b = input.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}

/// «Цифровой» эффект: три расстроенных копии, лёгкий статтер и квантование.
pub fn apply_voice_fx(input: &[f32], sample_rate: u32, strength: f32) -> Vec<f32> {
    if input.is_empty() || strength <= 0.0 {
        return input.to_vec();
    }
    let n = input.len();
    let sr = sample_rate as f32;
    let s = strength.clamp(0.0, 3.0);
    let mut out = input.to_vec();

    // Расстроенные копии с задержкой — «хорус» из оригинала.
    for (factor, delay_ms, gain) in [(1.012f32, 11.0f32, 0.32f32), (0.989, 19.0, 0.30), (1.024, 29.0, 0.18)] {
        let mut seed: u32 = 0x9E37_79B9;
        for i in 0..n {
            let t = i as f32;
            // Небольшое «дрожание» задержки.
            let wobble = (t / sr * 0.7 * std::f32::consts::TAU).sin() * 0.002 * sr;
            let pos = t * factor + wobble - delay_ms * sr / 1000.0;
            let idx = pos.floor();
            let v = if idx < 0.0 || idx >= (n - 1) as f32 {
                0.0
            } else {
                let i0 = idx as usize;
                let frac = pos - idx;
                let a = input[i0];
                let b = input[i0 + 1];
                a + (b - a) * frac
            };
            out[i] += v * gain * s;
        }
        seed = seed.wrapping_mul(31);
        let _ = seed;
    }

    // Квантование «как у дешёвого ЦАП».
    let crushed_mix = (0.22 * s * s).min(0.6);
    for sample in out.iter_mut() {
        let crushed = (*sample * 96.0).round() / 96.0;
        *sample = *sample * (1.0 - crushed_mix) + crushed * crushed_mix;
    }

    // Статтер: короткий кусок повторяется 2–3 раза (только на длинных фразах).
    if n as f32 > sr * 1.5 && s >= 0.7 {
        let slice_len = (sr * 0.065) as usize;
        if n > slice_len * 6 {
            let at = n / 2;
            let piece: Vec<f32> = out[at..at + slice_len].to_vec();
            let mut stuttered = Vec::with_capacity(out.len() + slice_len * 2);
            stuttered.extend_from_slice(&out[..at]);
            for rep in 0..3 {
                let damp = 0.9f32.powi(rep);
                stuttered.extend(piece.iter().map(|v| v * damp));
            }
            stuttered.extend_from_slice(&out[at..]);
            out = stuttered;
        }
    }
    out
}

/// Сделать WAV «глитча» — короткий цифровой треск при появлении.
pub fn make_glitch_sound(path: &Path, volume: u8) -> Result<(), String> {
    let sr = 22_050u32;
    let n = (sr as f32 * 0.22) as usize;
    let mut frames: Vec<i16> = Vec::with_capacity(n);
    let mut freq = 900.0f32;
    let mut noise_state: u32 = 42;
    for i in 0..n {
        if i % 600 == 0 {
            noise_state = noise_state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            freq = [300.0f32, 700.0, 1200.0, 1800.0, 2400.0][(noise_state >> 16) as usize % 5];
        }
        let env = (1.0 - i as f32 / n as f32).powf(1.5);
        let sq = if (std::f32::consts::TAU * freq * i as f32 / sr as f32).sin() > 0.0 { 1.0 } else { -1.0 };
        let noise = if (i / 400) % 3 == 0 {
            noise_state = noise_state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            ((noise_state >> 16) as f32 / 32_768.0) - 1.0
        } else {
            0.0
        };
        let v = 5500.0 * volume as f32 / 100.0 * env * (0.6 * sq + 0.5 * noise);
        frames.push(v.clamp(-32767.0, 32767.0) as i16);
    }
    write_wav_i16(path, &frames, sr).map_err(|e| format!("не смогла записать звук: {e}"))
}

/// Записать 16-битный моно-WAV.
pub fn write_wav_i16(path: &Path, samples: &[i16], sample_rate: u32) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let data_len = samples.len() as u32 * 2;
    let mut file = std::fs::File::create(path)?;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data_len).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?; // PCM
    file.write_all(&1u16.to_le_bytes())?; // моно
    file.write_all(&sample_rate.to_le_bytes())?;
    file.write_all(&(sample_rate * 2).to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?;
    file.write_all(&16u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_len.to_le_bytes())?;
    let mut buf = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    file.write_all(&buf)?;
    Ok(())
}

/// Записать сэмплы `f32` в WAV, нормализовав громкость (как `write_wav`).
pub fn write_wav(path: &Path, samples: &[f32], sample_rate: u32, volume: u8) -> std::io::Result<()> {
    if samples.is_empty() {
        return Ok(());
    }
    let peak = samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs())).max(1e-6);
    let gain = 0.92 * volume.min(100) as f32 / 100.0 / peak;
    let pcm: Vec<i16> = samples
        .iter()
        .map(|s| (s * gain * 32767.0).clamp(-32768.0, 32767.0) as i16)
        .collect();
    write_wav_i16(path, &pcm, sample_rate)
}

/// Приблизительная длительность сэмплов в секундах.
pub fn duration_of(samples: &[f32], sample_rate: u32) -> f32 {
    samples.len() as f32 / sample_rate.max(1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resamples_and_keeps_shape() {
        let input: Vec<f32> = (0..2400).map(|i| (i as f32 / 100.0).sin()).collect();
        let out = resample(&input, 24_000, 16_000);
        assert!((out.len() as i32 - 1600).abs() <= 2);
    }

    #[test]
    fn trims_leading_and_trailing_silence() {
        // Тишина по краям длиннее паузы-отступа (40 мс), иначе обрезать нечего.
        let mut input = vec![0.0f32; 20_000];
        input[9_000..10_000].iter_mut().for_each(|s| *s = 0.5);
        let out = trim_silence(&input, 24_000, 0.012);
        assert!(out.len() < input.len());
        assert!(out.len() >= 1_000);
        // Внутри осталось само слово.
        assert!(out.iter().any(|s| s.abs() > 0.4));
    }

    #[test]
    fn pitch_shift_changes_length() {
        let input: Vec<f32> = (0..1000).map(|i| (i as f32 / 50.0).sin()).collect();
        let up = pitch_shift(&input, 3.0);
        assert!(up.len() < input.len());
        assert!(!up.is_empty());
    }

    #[test]
    fn glitch_wav_is_written() {
        let dir = std::env::temp_dir().join("capri-test-glitch");
        let path = dir.join("glitch.wav");
        assert!(make_glitch_sound(&path, 80).is_ok());
        let bytes = std::fs::read(&path).expect("файл");
        assert_eq!(&bytes[0..4], b"RIFF");
        assert!(bytes.len() > 44);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fx_keeps_length_on_short_phrase() {
        let input: Vec<f32> = (0..500).map(|i| (i as f32 / 30.0).sin() * 0.5).collect();
        let out = apply_voice_fx(&input, FX_SR, 1.0);
        assert_eq!(out.len(), input.len());
    }
}
