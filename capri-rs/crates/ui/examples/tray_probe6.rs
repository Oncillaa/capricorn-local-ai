//! Что нам разрешено: `cargo run -p capri-ui --example tray_probe6`
//!
//! Пишет пробный файл в `%TEMP%` и в каталог программы и печатает результат.
//! Если запись в `%TEMP%` запрещена, значит процесс работает с урезанным
//! маркером доступа — это же объясняет и отказ области уведомлений с кодом 5.

use std::env;
use std::fs;

fn try_write(where_: &str, path: std::path::PathBuf) {
    match fs::write(&path, b"probe") {
        Ok(()) => {
            println!("{where_}: запись разрешена ({})", path.display());
            let _ = fs::remove_file(&path);
        }
        Err(why) => println!("{where_}: ЗАПРЕЩЕНА — {why}"),
    }
}

fn main() {
    for key in ["TEMP", "TMP", "USERPROFILE"] {
        println!("{key}={}", env::var(key).unwrap_or_else(|_| "—".into()));
    }

    let temp = env::temp_dir();
    try_write("%TEMP%", temp.join("capri_tray_probe.txt"));

    // Каталог рядом с программой и текущий каталог — для сравнения.
    try_write("текущий каталог", env::current_dir().unwrap_or_default().join("_probe_write.txt"));
}
