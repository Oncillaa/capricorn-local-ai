//! Панели Капри: окна с HTML-интерфейсом (tao + WebView2 через wry).
//!
//! Разделение обязанностей такое же, как во всём проекте:
//!
//! * аватар — нативное слоистое окно (`capri-ui`), его никто не трогает;
//! * панели (настройки, магазин, уровни, шахматы, перевод, терминал) — веб-вью
//!   в отдельном окне: дизайн со скриншотов делается на CSS, а не на GDI;
//! * общение с ядром — только через каналы [`Command`] (туда) и
//!   [`PanelEvent`] (обратно), как `UiEvent`/`UiRequest` у аватара.
//!
//! Ассеты (`web/`) вшиты в бинарник через `include_bytes!` и отдаются
//! собственным протоколом `capri://`, поэтому рядом с `capri.exe` ничего
//! раскладывать не нужно.
//!
//! Тред: своё окно живёт в том же треде, где создано, поэтому панели крутят
//! собственный цикл событий в отдельном потоке (`any_thread`), а главный поток
//! остаётся за окном аватара.

use std::sync::mpsc::{Receiver, Sender};

pub mod assets;

/// Панели, которые оболочка умеет открывать.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelId {
    Settings,
    Shop,
    Levels,
    Media,
    Chess,
    Chat,
    Translate,
    Terminal,
}

impl PanelId {
    pub const ALL: [PanelId; 8] = [
        PanelId::Settings,
        PanelId::Shop,
        PanelId::Levels,
        PanelId::Media,
        PanelId::Chess,
        PanelId::Chat,
        PanelId::Translate,
        PanelId::Terminal,
    ];

    /// Идентификатор для конфига и логов.
    pub fn as_str(self) -> &'static str {
        match self {
            PanelId::Settings => "settings",
            PanelId::Shop => "shop",
            PanelId::Levels => "levels",
            PanelId::Media => "media",
            PanelId::Chess => "chess",
            PanelId::Chat => "chat",
            PanelId::Translate => "translate",
            PanelId::Terminal => "terminal",
        }
    }

    /// Разобрать имя из конфига, меню или голосовой команды («магазин»).
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "settings" | "настройки" | "настройка" => Some(PanelId::Settings),
            "shop" | "магазин" => Some(PanelId::Shop),
            "levels" | "уровни" | "уровень" => Some(PanelId::Levels),
            "media" | "медиа" | "плеер" | "музыка" => Some(PanelId::Media),
            "chess" | "шахматы" | "игры" => Some(PanelId::Chess),
            "chat" | "чат" | "диалог" | "переписка" => Some(PanelId::Chat),
            "translate" | "перевод" | "переводчик" => Some(PanelId::Translate),
            "terminal" | "терминал" | "консоль" => Some(PanelId::Terminal),
            _ => None,
        }
    }

    /// Заголовок окна — как на скриншотах: `CAPRI // КАПРИ`.
    pub fn title(self) -> &'static str {
        match self {
            PanelId::Settings => "CAPRI // КАПРИ",
            PanelId::Shop => "CAPRI // МАГАЗИН",
            PanelId::Levels => "CAPRI // УРОВНИ",
            PanelId::Media => "CAPRI // МЕДИА",
            PanelId::Chess => "CAPRI // ИГРЫ - ШАХМАТЫ",
            PanelId::Chat => "CAPRI // ЧАТ",
            PanelId::Translate => "CAPRI // ПЕРЕВОД",
            PanelId::Terminal => "CAPRI // ТЕРМИНАЛ",
        }
    }

    /// Куда идти в `capri://`.
    pub fn page(self) -> &'static str {
        match self {
            PanelId::Settings => "settings.html",
            PanelId::Shop => "shop.html",
            PanelId::Levels => "levels.html",
            PanelId::Media => "media.html",
            PanelId::Chess => "chess.html",
            PanelId::Chat => "chat.html",
            PanelId::Translate => "translate.html",
            PanelId::Terminal => "terminal.html",
        }
    }

    /// Логический размер окна (CSS-пиксели, до масштаба DPI).
    pub fn size(self) -> (f64, f64) {
        match self {
            PanelId::Settings => (1060.0, 700.0),
            PanelId::Shop => (980.0, 660.0),
            PanelId::Levels => (980.0, 660.0),
            // Виджет узкий и невысокий — он живёт поверх плеера, а не заменяет его.
            PanelId::Media => (520.0, 470.0),
            PanelId::Chess => (1120.0, 720.0),
            // Чат повыше: в нём длинная лента сообщений и поле ввода внизу.
            PanelId::Chat => (860.0, 640.0),
            PanelId::Translate => (560.0, 260.0),
            PanelId::Terminal => (900.0, 520.0),
        }
    }
}

/// Что оболочка просит у панелей.
#[derive(Debug, Clone)]
pub enum Command {
    /// Показать панель (создать окно, если его ещё нет).
    Open(PanelId),
    /// Спрятать, не уничтожая (следующее открытие — мгновенное).
    Close(PanelId),
    /// Выполнить JS в панели — так ядро отдаёт состояние.
    Eval(PanelId, String),
    /// Открыть панель сразу в нужном разделе (`settings` → `reminders`).
    ///
    /// Выполнить JS прямо сейчас нельзя: окно могло ещё не загрузиться, и
    /// скрипт пропал бы. Поэтому просьба ждёт первого `Eval` — он приходит
    /// ровно тогда, когда страница попросила состояние.
    Section(PanelId, &'static str),
    /// Закрыть цикл событий (выход из программы).
    Quit,
}

/// Что панель сообщает оболочке.
#[derive(Debug, Clone)]
pub struct PanelEvent {
    pub panel: PanelId,
    /// `ready` — страница загрузилась, можно отдавать состояние;
    /// `close` — пользователь нажал Esc/крестик в интерфейсе;
    /// остальное — действия интерфейса (`open-config`, `set-scale`, …).
    pub action: String,
    /// Полезная нагрузка действия (может быть `null`).
    pub data: serde_json::Value,
}

/// Запустить цикл панелей. Блокируется до [`Command::Quit`] или закрытия.
///
/// Возвращает код выхода (0 — штатно).
pub fn run(commands: Receiver<Command>, events: Sender<PanelEvent>) -> i32 {
    #[cfg(windows)]
    {
        return imp::run(commands, events);
    }
    #[cfg(not(windows))]
    {
        let _ = (commands, events);
        log::warn!("Панели Капри доступны только на Windows");
        0
    }
}

#[cfg(windows)]
mod imp;
