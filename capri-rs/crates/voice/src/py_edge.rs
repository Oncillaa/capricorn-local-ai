//! Сетевой голос через Python-пакет `edge-tts`.
//!
//! Свой синтез на Rust ходит к сервису Microsoft напрямую и в этой сети
//! получает `403` (у службы поменялась схема токена). Python-пакет `edge-tts`
//! умеет то же самое, но с актуальным токеном — и, что важно, это обычный
//! дочерний процесс без COM: он работает там, где наши вызовы SAPI упираются
//! в чужие грабли.
//!
//! Пакет ставится один раз: `python -m pip install edge-tts`. Если питона или
//! пакета нет — честно возвращаем ошибку, Капри переходит к голосу Windows.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

/// Скрыть окно консоли у дочернего процесса.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Питон, в котором есть `edge_tts`. Ищем один раз на процесс.
fn python_command() -> Option<&'static str> {
    static FOUND: OnceLock<Option<&'static str>> = OnceLock::new();
    *FOUND.get_or_init(|| {
        for candidate in ["python", "py", "python3"] {
            let mut command = Command::new(candidate);
            command
                .args(["-c", "import edge_tts"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(CREATE_NO_WINDOW);
            }
            if let Ok(status) = command.status() {
                if status.success() {
                    log::info!("Сетевой голос: нашла edge-tts в «{candidate}»");
                    return Some(candidate);
                }
            }
        }
        log::warn!(
            "Сетевой голос через Python недоступен: нет питона или пакета edge-tts \
             (поставь: python -m pip install edge-tts)"
        );
        None
    })
}

/// Где писать рабочие файлы: первый каталог, куда реально можно писать.
fn work_dir(candidates: &[PathBuf]) -> Result<PathBuf, String> {
    for dir in candidates {
        if std::fs::create_dir_all(dir).is_ok() {
            return Ok(dir.clone());
        }
    }
    Err("некуда писать запись голоса".into())
}

/// Синтезировать фразу. Возвращает байты MP3.
pub fn synthesize(
    text: &str,
    voice: &str,
    rate: &str,
    pitch: &str,
    candidates: &[PathBuf],
) -> Result<Vec<u8>, String> {
    if text.trim().is_empty() {
        return Err("пустая фраза".into());
    }
    let python = python_command().ok_or("нет питона с edge-tts")?;
    let dir = work_dir(candidates)?;
    let text_path = dir.join("py-tts-text.txt");
    let media_path = dir.join("py-tts-out.mp3");
    let err_path = dir.join("py-tts-err.txt");
    std::fs::write(&text_path, text).map_err(|e| format!("текст для голоса не записался: {e}"))?;
    let _ = std::fs::remove_file(&media_path);

    let mut command = Command::new(python);
    command
        .arg("-m")
        .arg("edge_tts")
        .arg("--voice")
        .arg(voice)
        .arg("--file")
        .arg(&text_path)
        .arg("--write-media")
        .arg(&media_path);
    // Скорость и высота — в той же записи, что понимает сервис ("+10%", "+20Hz").
    if !rate.trim().is_empty() {
        command.arg("--rate").arg(rate.trim());
    }
    if !pitch.trim().is_empty() {
        command.arg("--pitch").arg(pitch.trim());
    }
    command.stdin(Stdio::null()).stdout(Stdio::null());
    let stderr = std::fs::File::create(&err_path).ok();
    match stderr {
        Some(file) => {
            command.stderr(file);
        }
        None => {
            command.stderr(Stdio::null());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let status = command
        .status()
        .map_err(|e| format!("питон для голоса не запустился: {e}"))?;
    if !status.success() {
        let why = read_tail(&err_path, 300);
        return Err(if why.is_empty() {
            format!("edge_tts не сработал (код {})", status.code().unwrap_or(-1))
        } else {
            format!("edge_tts: {why}")
        });
    }
    let bytes = std::fs::read(&media_path).map_err(|e| format!("запись голоса не прочиталась: {e}"))?;
    let _ = std::fs::remove_file(&text_path);
    if bytes.len() < 100 {
        return Err("edge_tts вернул пустой звук".into());
    }
    Ok(bytes)
}

/// Последние `limit` символов файла — для понятной ошибки.
fn read_tail(path: &Path, limit: usize) -> String {
    let Ok(text) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let tail: String = text.chars().skip(text.chars().count() - limit).collect();
    format!("…{tail}")
}
