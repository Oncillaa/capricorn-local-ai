//! Проба: сетевой голос через Python-пакет edge-tts из процесса Rust.
//!
//! Запуск: `cargo run -p capri-voice --example py_tts_probe`

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let dir = PathBuf::from("C:\\Users\\Oncilla\\Desktop\\capricorn-main\\capri-rs\\target");
    let text = dir.join("py-tts-text.txt");
    let out = dir.join("py-tts-out.mp3");
    let log = dir.join("py-tts-err.txt");
    std::fs::write(&text, "Привет! Это Капри, проверка сетевого голоса.").ok();
    let _ = std::fs::remove_file(&out);

    let stderr = std::fs::File::create(&log).expect("файл ошибок");
    let status = Command::new("python")
        .arg("-m")
        .arg("edge_tts")
        .arg("--voice")
        .arg("ru-RU-SvetlanaNeural")
        .arg("--file")
        .arg(&text)
        .arg("--write-media")
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .stderr(stderr)
        .status();
    println!("статус: {status:?}");
    println!("mp3: {:?}", std::fs::metadata(&out).map(|m| m.len()));
    if let Ok(log) = std::fs::read_to_string(&log) {
        let tail: String = log.chars().rev().take(600).collect::<String>().chars().rev().collect();
        println!("--- stderr ---\n{tail}\n---");
    }
    let _ = std::fs::write(
        dir.join("py_tts_probe_result.txt"),
        format!("{status:?} / {:?}", std::fs::metadata(&out).map(|m| m.len())),
    );
}
