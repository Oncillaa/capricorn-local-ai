//! Окно аватара, меню в трее и диалоги.
//!
//! Капри — прозрачное окно с фигурой и подписью, иконка в трее и меню по
//! правой кнопке. Всё общение с ядром идёт через каналы [`UiEvent`] и
//! [`UiRequest`], поэтому окно можно проверить и без запуска Windows-кода.

use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use parking_lot::RwLock;

pub mod canvas;
pub mod sprite;
pub mod text;

#[cfg(windows)]
pub mod avatar;

/// Что ядро говорит окну аватара.
#[derive(Debug, Clone)]
pub enum UiEvent {
    /// Появиться с глитч-вспышкой.
    Summon,
    /// Исчезнуть.
    Vanish,
    /// Режим и «сколько секунд ещё слушаю»: "idle" / "listening" / "thinking" / "speaking".
    Mode(String, f32),
    /// Показать текст (реплика Капри, «> команда», служебная строка).
    Subtitle(String),
    /// Проиграть анимацию: "capri" (машет) или "eyes" (думает).
    PlayAnim(String),
    /// Пройтись по экрану.
    Roam,
    /// Прыгнуть молнией в другое место.
    Hop,
    /// Перейти в названный угол ("left", "right", "top", "bottom", "center").
    MoveTo(String),
    /// Перекрасить аватар в цвет скина (`None` — исходный вид).
    SetSkin(Option<[u8; 3]>),
}

/// Что окно просит у ядра.
#[derive(Debug, Clone)]
pub enum UiRequest {
    /// Готовая команда (из речи или текста).
    Command(String),
    /// «Замолчать» — прервать речь.
    Interrupt,
    /// «Позвать Капри».
    Call,
    Quit,
    SetMic(Option<String>),
    MicNext,
    SetVolume(u8),
    SetVoiceFx(f32),
    SetChatter(bool),
    SetSearch(bool),
    /// Режим обращения, имя, пол.
    SetProfile(String, String, String),
    /// Перечитать config.json.
    Reload,
    /// Открыть config.json в блокноте.
    OpenConfig,
    /// Надеть скин.
    SelectSkin(String),
}

/// Что нужно окну от оболочки.
#[derive(Clone)]
pub struct UiOptions {
    pub base_dir: PathBuf,
    pub assets_dir: PathBuf,
    /// Текущий уровень микрофона 0…10000 (для полоски в настройках микрофона).
    pub mic_level: Arc<AtomicU32>,
    /// Текущий микрофон (для галочки в меню).
    pub current_mic: Arc<RwLock<Option<String>>>,
}

/// Запустить UI в текущем (главном) потоке и блокироваться до выхода.
/// Возвращает код выхода.
pub fn run(opts: UiOptions, _cfg: Arc<RwLock<capri_core::Config>>, events: Receiver<UiEvent>, requests: Sender<UiRequest>) -> i32 {
    #[cfg(windows)]
    {
        // События из ядра раскладываем в окно из отдельного потока:
        // главный поток занят циклом сообщений Windows.
        let sprites = sprite::Sprites::load(&opts.assets_dir, &opts.base_dir);
        if sprites.is_empty() {
            log::warn!(
                "Не нашла картинки Капри в {} — будет пустое окно",
                opts.assets_dir.display()
            );
        }

        let requests_for_events = requests.clone();
        std::thread::spawn(move || {
            while let Ok(event) = events.recv() {
                avatar::push_event(event);
            }
            let _ = requests_for_events.send(UiRequest::Quit);
        });

        avatar::run(sprites, requests)
    }
    #[cfg(not(windows))]
    {
        let _ = (&opts, &events, &requests);
        log::error!("Окно Капри доступно только на Windows");
        1
    }
}

/// Скины, которые можно надеть (как `skins` в `save_data.json`).
pub fn skins() -> Vec<(&'static str, &'static str, Option<[u8; 3]>)> {
    vec![
        // `purple` — исходный вид, перекраски нет (`None`).
        ("purple", "Фиолетовый", None),
        ("neon", "Неон", Some([40, 255, 150])),
        ("red", "Алый", Some([255, 34, 96])),
        ("ice", "Лёд", Some([100, 200, 255])),
        ("toxic", "Токсин", Some([200, 255, 40])),
        ("sunset", "Закат", Some([255, 140, 0])),
    ]
}

/// Цвет скина по имени. `None` — либо скин исходный, либо имени нет.
pub fn skin_color(name: &str) -> Option<[u8; 3]> {
    skins().into_iter().find(|(id, _, _)| *id == name).and_then(|(_, _, color)| color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_skin_colors() {
        assert!(skin_color("neon").is_some());
        assert!(skin_color("нет такого").is_none());
        // У исходного скина перекраски нет — окно должно остаться как есть.
        assert!(skin_color("purple").is_none());
        assert_eq!(skins().len(), 6);
    }
}
