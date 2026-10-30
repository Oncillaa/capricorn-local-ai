//! Капри 2.0 — голосовой помощник с характером.
//!
//! Запуск: `capri.exe` рядом с `config.json`, `assets/` и `models/`.
//! Подробности — в `README.md`.

// В релизе — без чёрного окна консоли: это настольный питомец, а не утилита.
// В отладочной сборке консоль остаётся, иначе не видно паник и `println!`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capri;
mod logger;
mod wake;

use std::panic::PanicHookInfo;
use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::sync::mpsc::channel;
use std::sync::Arc;

use capri_core::persona;
use capri_core::{Config, Platform};
use capri_ui::{UiEvent, UiOptions, UiRequest};
use capri_win::WindowsPlatform;
use parking_lot::RwLock;

use crate::capri::{Capri, Deps};
use crate::logger as log;

/// Сколько уровней вверх смотрим в поисках `config.json`.
const MAX_UP: usize = 5;

fn main() -> std::process::ExitCode {
    let console = std::env::args().any(|arg| arg == "--console");
    if console {
        // Собраны как оконное приложение — просим консоль у родителя,
        // иначе `--console` печатал бы в никуда.
        attach_console();
    }

    let base_dir = find_base_dir();
    log::init(&base_dir, console);

    // Всё, что упало, — в лог по-русски, а не в никуда.
    std::panic::set_hook(Box::new(panic_hook));

    log::write_line(
        "INFO",
        &format!(
            "Капри 2.0 запускается, каталог: {}, лог: {}",
            base_dir.display(),
            log::log_path().display()
        ),
    );

    let code = run(base_dir);
    log::write_line("INFO", &format!("Капри 2.0 завершилась, код {code}"));
    std::process::ExitCode::from(code as u8)
}

/// Взять консоль у запустившего процесса — для режима `--console`.
fn attach_console() {
    #[cfg(windows)]
    {
        use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn panic_hook(info: &PanicHookInfo) {
    let where_ = info
        .location()
        .map(|l| format!("{}:{}", l.file(), l.line()))
        .unwrap_or_else(|| "неизвестно".into());
    log::write_line("ERROR", &format!("Непредвиденная ошибка в {where_}: {info}"));
}

/// Каталог, где лежат `config.json`, `assets/`, `models/`.
///
/// Порядок: папка программы, её родители (до [`MAX_UP`] уровней — так
/// `target\release\capri.exe` находит настройки в корне проекта), затем
/// текущий каталог. И двойной клик, и запуск из терминала дают одно и то же.
fn find_base_dir() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.to_path_buf()));

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = &exe_dir {
        candidates.push(dir.clone());
        let mut up = dir.parent();
        for _ in 0..MAX_UP {
            match up {
                Some(path) => {
                    candidates.push(path.to_path_buf());
                    up = path.parent();
                }
                None => break,
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }

    for candidate in &candidates {
        if candidate.join("config.json").is_file() {
            return candidate.clone();
        }
    }
    // Файла настроек ещё нет — годится каталог с ресурсами.
    for candidate in &candidates {
        if candidate.join("assets").is_dir() {
            return candidate.clone();
        }
    }
    // Совсем пусто — берём папку программы (или текущую).
    candidates
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from("."))
}

fn run(base_dir: PathBuf) -> i32 {
    let config_path = base_dir.join("config.json");
    let config = Config::load(&config_path);
    if let Some(warning) = &config.load_warning {
        log::write_line("WARN", &format!("config.json: {warning}"));
    }
    if !config_path.is_file() {
        log::write_line(
            "WARN",
            "config.json не найден — работаю со значениями по умолчанию",
        );
        if let Err(e) = config.save() {
            log::write_line("WARN", &format!("Не смогла создать config.json: {e}"));
        }
    }

    // Обращение к пользователю знает вся персона.
    persona::set_user(config.user_name.clone(), &config.user_gender);

    let platform: Arc<dyn Platform> = Arc::new(WindowsPlatform::new(base_dir.clone()));
    let cfg = Arc::new(RwLock::new(config));
    let speaker = Arc::new(capri_voice::Speaker::new(
        &cfg.read(),
        &capri_voice::cache_root(),
    ));

    let (events_tx, events_rx) = channel::<UiEvent>();
    let (requests_tx, requests_rx) = channel::<UiRequest>();

    let mic_level = Arc::new(AtomicU32::new(0f32.to_bits()));
    let current_mic = Arc::new(RwLock::new(None::<String>));

    let deps = Deps {
        base_dir: base_dir.clone(),
        cfg: Arc::clone(&cfg),
        platform,
        speaker,
        events: events_tx,
        mic_level: Arc::clone(&mic_level),
        current_mic: Arc::clone(&current_mic),
    };

    let (capri, jobs, _bridge) = Capri::start(deps, requests_rx);

    // Ядро работает в своём потоке, окно — в главном: Windows требует,
    // чтобы окно жило именно в том потоке, который его создал.
    let worker = std::thread::spawn(move || {
        let mut capri = capri;
        capri.run(jobs)
    });

    let options = UiOptions {
        assets_dir: base_dir.join("assets"),
        base_dir,
        mic_level,
        current_mic,
    };

    let code = capri_ui::run(options, Arc::clone(&cfg), events_rx, requests_tx);

    // Окно закрылось — просим ядро закончить и не держим поток вечно.
    log::write_line("INFO", "Окно закрыто");
    if worker.join().is_err() {
        log::write_line("WARN", "Поток ядра завершился с ошибкой");
    }
    code
}
