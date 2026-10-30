//! Всё, что ядро просит у Windows. Реализация — в крейте `capri-win`.
//!
//! У каждого метода есть тело по умолчанию: если платформа чего-то не умеет,
//! команда просто честно ответит «не получилось», а не упадёт.

use std::path::PathBuf;

pub type Res<T> = Result<T, String>;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub app: String,
    /// Длительность и позиция, если плеер их отдаёт (секунды).
    pub duration: Option<f64>,
    pub position: Option<f64>,
    pub playing: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: String,
    pub cpu: f32,
    pub mem_mb: f32,
}

#[derive(Debug, Clone, Default)]
pub struct WindowInfo {
    pub handle: isize,
    pub title: String,
    pub class: String,
    pub exe: Option<String>,
    pub pid: u32,
}

#[derive(Debug, Clone, Default)]
pub struct InstalledProgram {
    pub name: String,
    pub target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaCommand {
    PlayPause,
    Play,
    Pause,
    Next,
    Prev,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerMode {
    Shutdown,
    Restart,
    Sleep,
    Hibernate,
    Logoff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabCommand {
    Close,
    New,
    ReopenClosed,
    Next,
    Prev,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Down,
    Up,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyStroke {
    /// Виртуальный код клавиши (VK_*).
    pub vk: u16,
    /// «Расширенная» клавиша (стрелки, Win, Delete…) — нужен флаг KEYEVENTF_EXTENDEDKEY.
    pub extended: bool,
}

impl KeyStroke {
    pub fn new(vk: u16) -> Self {
        Self { vk, extended: false }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PcReport {
    pub cpu_name: String,
    pub cpu_load: f32,
    pub ram_total_gb: f32,
    pub ram_used_gb: f32,
    pub gpu_name: String,
    pub disks: Vec<(String, f32, f32)>, // (имя, свободно ГБ, всего ГБ)
    pub uptime_hours: f32,
    pub problems: Vec<String>,
    pub advice: Vec<String>,
}

/// Платформенный слой. Все методы должны быть быстрыми или кэшировать
/// результат: их зовут из горячего пути разбора команд.
pub trait Platform: Send + Sync {
    // ---------- запуск ----------
    fn open_url(&self, url: &str, browser: Option<&str>) -> Res<()> {
        let _ = browser;
        Err(format!("открыть {url} не получилось"))
    }

    /// Запустить цель: exe, `shell:`-URI, `ms-settings:`, путь или URL.
    fn launch(&self, target: &str) -> Res<()> {
        Err(format!("не нашла «{target}»"))
    }

    fn run_powershell(&self, script: &str) -> Res<String> {
        let _ = script;
        Err("PowerShell недоступен".into())
    }

    // ---------- поиск ----------
    fn installed_programs(&self) -> Vec<InstalledProgram> {
        Vec::new()
    }

    fn find_browser(&self, name: &str) -> Option<PathBuf> {
        let _ = name;
        None
    }

    fn known_folder(&self, key: &str) -> Option<PathBuf> {
        let _ = key;
        None
    }

    fn search_file(&self, name: &str) -> Option<PathBuf> {
        let _ = name;
        None
    }

    fn audio_input_devices(&self) -> Vec<String> {
        Vec::new()
    }

    // ---------- звук и яркость ----------
    fn get_volume(&self) -> Option<u8> {
        None
    }
    fn set_volume(&self, percent: u8) -> Res<()> {
        let _ = percent;
        Err("громкость недоступна".into())
    }
    fn set_mute(&self, on: bool) -> Res<()> {
        let _ = on;
        Err("звук недоступен".into())
    }
    fn get_brightness(&self) -> Option<u8> {
        None
    }
    fn set_brightness(&self, percent: u8) -> Res<()> {
        let _ = percent;
        Err("яркость недоступна".into())
    }

    // ---------- окна ----------
    fn foreground_window(&self) -> Option<WindowInfo> {
        None
    }
    fn window_by_title(&self, query: &str) -> Option<WindowInfo> {
        let _ = query;
        None
    }
    fn window_by_process(&self, process: &str) -> Option<WindowInfo> {
        let _ = process;
        None
    }
    fn list_windows(&self) -> Vec<WindowInfo> {
        Vec::new()
    }
    fn focus_window(&self, handle: isize) -> Res<()> {
        let _ = handle;
        Err("не смогла переключиться на окно".into())
    }
    fn close_window(&self, handle: isize) -> Res<()> {
        let _ = handle;
        Err("не смогла закрыть окно".into())
    }
    fn minimize_window(&self, handle: isize) -> Res<()> {
        let _ = handle;
        Err("не смогла свернуть окно".into())
    }
    fn minimize_all(&self) -> Res<()> {
        Err("не смогла свернуть всё".into())
    }
    /// Адрес открытой страницы в браузере (для «запомни этот сайт»).
    fn window_url(&self, handle: isize) -> Option<String> {
        let _ = handle;
        None
    }
    /// Заголовок активной вкладки/страницы (для поиска «что играет»).
    fn window_title(&self, handle: isize) -> Option<String> {
        let _ = handle;
        None
    }

    // ---------- ввод ----------
    /// Зажать клавиши по порядку и отпустить в обратном — так работают
    /// сочетания вроде Win+Tab.
    fn send_keys(&self, keys: &[KeyStroke]) -> Res<()> {
        let _ = keys;
        Err("ввод с клавиатуры недоступен".into())
    }
    fn type_text(&self, text: &str) -> Res<()> {
        let _ = text;
        Err("ввод с клавиатуры недоступен".into())
    }
    fn press_key(&self, vk: u16) -> Res<()> {
        self.send_keys(&[KeyStroke::new(vk)])
    }
    fn scroll(&self, dir: ScrollDirection, steps: u32) -> Res<()> {
        let _ = (dir, steps);
        Err("прокрутка недоступна".into())
    }
    fn tab_command(&self, cmd: TabCommand) -> Res<()> {
        let _ = cmd;
        Err("вкладки недоступны".into())
    }
    fn clipboard_get(&self) -> Option<String> {
        None
    }
    fn clipboard_set(&self, text: &str) -> Res<()> {
        let _ = text;
        Err("буфер обмена недоступен".into())
    }

    // ---------- мультимедиа ----------
    fn media(&self, cmd: MediaCommand) -> Res<()> {
        let _ = cmd;
        Err("плеер не отвечает".into())
    }
    fn now_playing(&self) -> Option<Track> {
        None
    }
    fn media_seek(&self, seconds: f64) -> Res<()> {
        let _ = seconds;
        Err("перемотка недоступна".into())
    }

    // ---------- процессы ----------
    fn processes(&self) -> Vec<ProcInfo> {
        Vec::new()
    }
    fn kill_process(&self, name: &str, force: bool) -> usize {
        let _ = (name, force);
        0
    }

    // ---------- система ----------
    fn screenshot(&self) -> Res<PathBuf> {
        Err("скриншот не получился".into())
    }
    fn lock_workstation(&self) -> Res<()> {
        Err("не смогла заблокировать".into())
    }
    fn power(&self, mode: PowerMode) -> Res<()> {
        let _ = mode;
        Err("не смогла".into())
    }
    fn analyze_pc(&self) -> PcReport {
        PcReport::default()
    }
    fn user_temp_dir(&self) -> PathBuf {
        std::env::temp_dir()
    }
    fn base_dir(&self) -> PathBuf {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }
    /// Сворачивает ли сейчас полноэкранное приложение (игры, видео) —
    /// чтобы Капри не вылезала поверх фильма.
    fn fullscreen_app_active(&self) -> bool {
        false
    }
    /// Отправить сообщение в Telegram Desktop: открыть чат по имени и напечатать.
    fn telegram_send(&self, chat: &str, text: &str, press_enter: bool) -> Res<()> {
        let _ = (chat, text, press_enter);
        Err("Telegram не найден".into())
    }
}
