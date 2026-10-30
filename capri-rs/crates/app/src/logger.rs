//! Лог в файл `capri.log` рядом с программой.
//!
//! Здесь же ставится приёмник для крейта [`log`]: все `log::info!` из любого
//! модуля попадают в один файл. Отдельная библиотека логирования не нужна,
//! поэтому всё написано руками — зато точно знаем, куда что пишется.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use parking_lot::Mutex;

struct Sink {
    file: Option<Mutex<File>>,
    /// Путь к файлу — нужен для сообщения об ошибке.
    path: PathBuf,
    /// Печатать ли в консоль (при запуске из терминала).
    console: bool,
}

static SINK: OnceLock<Sink> = OnceLock::new();

/// Приёмник для крейта `log` — перекладывает сообщения в наш файл.
struct Logger;

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let level = match record.level() {
            log::Level::Error => "ERROR",
            log::Level::Warn => "WARN",
            log::Level::Info => "INFO",
            log::Level::Debug => "DEBUG",
            log::Level::Trace => "TRACE",
        };
        write_line(level, &record.args().to_string());
    }

    fn flush(&self) {
        if let Some(sink) = SINK.get() {
            if let Some(file) = &sink.file {
                let _ = file.lock().flush();
            }
        }
    }
}

/// Включить запись в `capri.log` и подключить приёмник `log`.
pub fn init(base_dir: &Path, console: bool) {
    let path = base_dir.join("capri.log");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()
        .map(Mutex::new);
    let _ = SINK.set(Sink { file, path, console });

    // Уровень Debug полезен при разборе жалоб, но по умолчанию — Info.
    let level = if console { log::LevelFilter::Debug } else { log::LevelFilter::Info };
    if log::set_logger(&Logger).is_ok() {
        log::set_max_level(level);
    }
}

/// Куда пишем — пригодится в сообщении об ошибке.
pub fn log_path() -> PathBuf {
    SINK.get().map(|sink| sink.path.clone()).unwrap_or_default()
}

pub fn write_line(level: &str, message: &str) {
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let line = format!("[{stamp}] {level:<5} {message}\n");
    if let Some(sink) = SINK.get() {
        if sink.console {
            // Кириллица в консоли Windows иногда ломается — это не критично.
            print!("{line}");
        }
        if let Some(file) = &sink.file {
            let mut guard = file.lock();
            let _ = guard.write_all(line.as_bytes());
            let _ = guard.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_through_the_log_crate() {
        let dir = std::env::temp_dir().join("capri-log-test");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        init(&dir, false);
        log::info!("проверка связи");
        let text = std::fs::read_to_string(dir.join("capri.log")).unwrap_or_default();
        assert!(text.contains("проверка связи"), "в логе: {text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
