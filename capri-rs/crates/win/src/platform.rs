//! `Platform` для Windows: тонкая склейка модулей.

use std::path::PathBuf;

use capri_core::platform::{
    InstalledProgram, KeyStroke, MediaCommand, PcReport, Platform, PowerMode, ProcInfo, Res,
    ScrollDirection, TabCommand, Track, WindowInfo,
};

use crate::{audio, input, launch, media, screenshot, system, windows_api};

pub struct WindowsPlatform {
    base_dir: PathBuf,
}

impl WindowsPlatform {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }
}

impl Platform for WindowsPlatform {
    fn base_dir(&self) -> PathBuf {
        self.base_dir.clone()
    }

    // ---------- запуск ----------
    fn open_url(&self, url: &str, browser: Option<&str>) -> Res<()> {
        let exe = browser
            .and_then(launch::find_browser)
            .or_else(|| launch::find_browser(&self.browser_setting()));
        launch::open_url(url, exe.as_deref())
    }

    fn launch(&self, target: &str) -> Res<()> {
        launch::launch_smart(target)
    }

    fn run_powershell(&self, script: &str) -> Res<String> {
        crate::util::ps(script, 60)
    }

    // ---------- поиск ----------
    fn installed_programs(&self) -> Vec<InstalledProgram> {
        launch::installed_programs()
    }

    fn find_browser(&self, name: &str) -> Option<PathBuf> {
        launch::find_browser(name)
    }

    fn known_folder(&self, key: &str) -> Option<PathBuf> {
        launch::known_folder(key)
    }

    fn search_file(&self, name: &str) -> Option<PathBuf> {
        launch::search_file(name)
    }

    fn audio_input_devices(&self) -> Vec<String> {
        launch::audio_input_devices()
    }

    // ---------- звук и яркость ----------
    fn get_volume(&self) -> Option<u8> {
        audio::get_volume()
    }

    fn set_volume(&self, percent: u8) -> Res<()> {
        audio::set_volume(percent)
    }

    fn set_mute(&self, on: bool) -> Res<()> {
        audio::set_mute(on)
    }

    fn get_brightness(&self) -> Option<u8> {
        audio::get_brightness()
    }

    fn set_brightness(&self, percent: u8) -> Res<()> {
        audio::set_brightness(percent)
    }

    // ---------- окна ----------
    fn foreground_window(&self) -> Option<WindowInfo> {
        windows_api::foreground()
    }

    fn window_by_title(&self, query: &str) -> Option<WindowInfo> {
        windows_api::find_by_title(query)
    }

    fn window_by_process(&self, process: &str) -> Option<WindowInfo> {
        windows_api::find_by_process(process)
    }

    fn list_windows(&self) -> Vec<WindowInfo> {
        windows_api::list()
    }

    fn focus_window(&self, handle: isize) -> Res<()> {
        windows_api::focus(handle)
    }

    fn close_window(&self, handle: isize) -> Res<()> {
        windows_api::close(handle)
    }

    fn minimize_window(&self, handle: isize) -> Res<()> {
        windows_api::minimize(handle)
    }

    fn minimize_all(&self) -> Res<()> {
        windows_api::minimize_all()
    }

    fn window_url(&self, handle: isize) -> Option<String> {
        windows_api::window_url(handle)
    }

    fn window_title(&self, handle: isize) -> Option<String> {
        windows_api::title_of(handle)
    }

    // ---------- ввод ----------
    fn send_keys(&self, keys: &[KeyStroke]) -> Res<()> {
        input::send_keys(keys)
    }

    fn type_text(&self, text: &str) -> Res<()> {
        input::type_text(text)
    }

    fn scroll(&self, dir: ScrollDirection, steps: u32) -> Res<()> {
        input::scroll(dir, steps)
    }

    fn tab_command(&self, cmd: TabCommand) -> Res<()> {
        input::tab_command(cmd)
    }

    fn clipboard_get(&self) -> Option<String> {
        system::clipboard_get()
    }

    fn clipboard_set(&self, text: &str) -> Res<()> {
        system::clipboard_set(text)
    }

    // ---------- мультимедиа ----------
    fn media(&self, cmd: MediaCommand) -> Res<()> {
        media::media_command(cmd)
    }

    fn now_playing(&self) -> Option<Track> {
        media::now_playing()
    }

    fn media_seek(&self, seconds: f64) -> Res<()> {
        media::seek(seconds)
    }

    // ---------- процессы ----------
    fn processes(&self) -> Vec<ProcInfo> {
        system::processes()
    }

    fn kill_process(&self, name: &str, force: bool) -> usize {
        system::kill_process(name, force)
    }

    // ---------- система ----------
    fn screenshot(&self) -> Res<PathBuf> {
        screenshot::capture_screen()
    }

    fn lock_workstation(&self) -> Res<()> {
        system::lock_workstation()
    }

    fn power(&self, mode: PowerMode) -> Res<()> {
        system::power(mode)
    }

    fn analyze_pc(&self) -> PcReport {
        system::analyze_pc(&self.base_dir)
    }

    fn user_temp_dir(&self) -> PathBuf {
        std::env::temp_dir()
    }

    fn fullscreen_app_active(&self) -> bool {
        windows_api::fullscreen_active()
    }

    fn telegram_send(&self, chat: &str, text: &str, press_enter: bool) -> Res<()> {
        system::telegram_send(chat, text, press_enter)
    }
}

impl WindowsPlatform {
    /// Браузер из настроек читается движком, но `open_url` умеет и сам —
    /// берём переменную окружения как подсказку, если движок её выставил.
    fn browser_setting(&self) -> String {
        std::env::var("CAPRI_BROWSER").unwrap_or_default()
    }
}
