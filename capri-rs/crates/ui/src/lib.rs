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
pub mod hotkey;
pub mod menu_ring;
pub mod sprite;
pub mod text;
pub mod view;

#[cfg(windows)]
pub mod avatar;
#[cfg(windows)]
pub mod layered;
#[cfg(windows)]
pub mod radial;
pub mod select;

/// Класс окна аватара: по нему второй экземпляр находит уже запущенную Капри.
pub const AVATAR_CLASS: &str = "CapriAvatarWindow";

/// Сообщение «покажись» — его шлёт второй экземпляр первому.
///
/// `WM_APP` в Win32 равно `0x8000`; пишем числом, чтобы константа жила без
/// зависимости от Windows-крейта.
pub const WM_CAPRI_SUMMON: u32 = 0x8000 + 3;

/// Изменение внешнего вида — приходит из панели настроек.
#[derive(Debug, Clone, PartialEq)]
pub enum CharacterSetting {
    /// Размер в процентах, 1…100.
    Scale(u8),
    /// Живость: `calm`, `normal` или `lively`.
    Liveliness(String),
    /// Номер монитора, на котором жить.
    Monitor(u32),
}

/// Изменение в разделе «Поведение» — тоже из панели.
#[derive(Debug, Clone, PartialEq)]
pub enum BehaviorSetting {
    /// Перебивать ли Капри на полуслове (`barge_in`).
    BargeIn(bool),
    /// Здороваться при запуске (`greet_on_start`).
    GreetOnStart(bool),
    /// Болталка: сама заговаривает, когда скучно (`chatter`).
    Chatter(bool),
    /// Как часто заговаривать — интервал в минутах (`chatter_min_minutes`,
    /// `chatter_max_minutes`).
    ChatterMinutes(f32, f32),
    /// Шанс прогулки по экрану, 0…100 (`roam_chance`).
    RoamChance(f32),
    /// Искать в сети, когда не уверена (`search_when_unsure`).
    SearchWhenUnsure(bool),
    /// Язык, на который переводить (`translate_to`): `ru`, `en`, …
    TranslateTo(String),
    /// Адрес LibreTranslate (`translate_libretranslate_url`); пусто — не использовать.
    TranslateUrl(String),
}

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
    /// Сменить размер, живость или монитор — на лету, без перезапуска.
    Character(CharacterSetting),
    /// Перерегистрировать горячую клавишу (изменили в config.json).
    Hotkey(String),
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
    /// Капри погладили — клик по фигуре без перетаскивания.
    Petted,
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
    /// Открыть панель интерфейса (`"settings"`, `"shop"`, …).
    OpenPanel(String),
    /// Надеть скин.
    SelectSkin(String),
    /// Только перекрасить фигуру: прогресс уже записан оболочкой.
    SkinColor(Option<[u8; 3]>),
    /// Сменить размер, живость или монитор.
    Character(CharacterSetting),
    /// Изменить характер: болталка, перебивание, прогулки.
    Behavior(BehaviorSetting),
    /// Выбран пункт радиального меню (идентификатор сектора).
    RadialAction(String),
    /// Горячая клавиша не встала: причина для человека.
    HotkeyError(String),
    /// Положить текст в буфер обмена (кнопка «Отправить в main»).
    CopyToClipboard(String),
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
    /// Мониторы: где живёт окно и куда можно гулять.
    pub monitors: Vec<capri_core::platform::MonitorInfo>,
}

/// Запустить UI в текущем (главном) потоке и блокироваться до выхода.
/// Возвращает код выхода.
pub fn run(
    opts: UiOptions,
    cfg: Arc<RwLock<capri_core::Config>>,
    events: Receiver<UiEvent>,
    requests: Sender<UiRequest>,
) -> i32 {
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

        // Размер, живость и монитор берём из настроек: окно должно выглядеть
        // так, как записано в config.json, ещё до первого действия пользователя.
        let view = view::ViewSettings::from_config(&cfg.read());
        let hotkey = cfg.read().hotkey_menu.clone();

        let requests_for_events = requests.clone();
        std::thread::spawn(move || {
            while let Ok(event) = events.recv() {
                avatar::push_event(event);
            }
            let _ = requests_for_events.send(UiRequest::Quit);
        });

        avatar::run(sprites, requests, view, opts.monitors, hotkey)
    }
    #[cfg(not(windows))]
    {
        let _ = (&opts, &events, &requests, &cfg);
        log::error!("Окно Капри доступно только на Windows");
        1
    }
}

/// Цвет скина по имени. `None` — либо скин исходный, либо имени нет.
///
/// Каталог живёт в одном месте — [`capri_core::save::SKINS`]. Раньше здесь
/// лежала вторая копия списка со старыми идентификаторами (`red` вместо
/// `lada`, без `shadow`), и она тихо расходилась с настоящим каталогом.
pub fn skin_color(name: &str) -> Option<[u8; 3]> {
    capri_core::save::skin_color(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skin_colors_come_from_the_single_catalog() {
        // Цвета совпадают с каталогом прогресса — второго списка больше нет.
        for skin in capri_core::save::SKINS {
            assert_eq!(
                skin_color(skin.id),
                skin.color,
                "скин «{}» разошёлся с каталогом",
                skin.id
            );
        }
        assert!(skin_color("toxic").is_some());
        assert!(skin_color("нет такого").is_none());
        // У исходного скина перекраски нет — окно должно остаться как есть.
        assert!(skin_color("purple").is_none());
        // Старое имя первых версий понимается как «Лада».
        assert_eq!(skin_color("red"), capri_core::save::skin_color("lada"));
    }
}
