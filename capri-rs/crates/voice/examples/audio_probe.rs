//! Проба вывода звука: слышно ли вообще и чем играет.
//!
//! Запуск: `cargo run -p capri-voice --example audio_probe`
//!
//! Печатает, удалось ли открыть устройство вывода и сколько времени заняло
//! воспроизведение. Нужно, чтобы отделить «звук не работает» от «SAPI сбоит».

use std::time::Instant;

fn main() {
    let rate = 22_050u32;
    let seconds = 0.7f32;
    let count = (rate as f32 * seconds) as usize;
    let samples: Vec<f32> = (0..count)
        .map(|i| {
            let t = i as f32 / rate as f32;
            0.25 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
        })
        .collect();

    let started = Instant::now();
    let played = capri_voice::audio::play_samples(samples, rate, 60);
    let took = started.elapsed();
    println!("play_samples вернул {played} за {took:?}");

    let started = Instant::now();
    let wav = std::env::temp_dir().join("capri-audio-probe.wav");
    match capri_voice::audio::write_wav(&wav, &(0..count).map(|_| 0.0f32).collect::<Vec<f32>>(), rate, 60) {
        Ok(()) => println!("write_wav ок: {wav:?}"),
        Err(e) => println!("write_wav не смог: {e}"),
    }
    println!("запись заняла {:?}", started.elapsed());
}
