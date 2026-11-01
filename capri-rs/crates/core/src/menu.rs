//! Пункты радиального меню.
//!
//! Список один на всех: по нему рисуется кольцо (`ui::menu_ring`), по нему же
//! ядро решает, что делать с выбранным сектором. Держим в ядре, чтобы порядок
//! секторов и подписи не разъехались между окном и обработчиком.
//!
//! Колец два. Основное — [`SECTORS`]. Сектор «Инструменты» не открывает панель,
//! а показывает второе кольцо — [`TOOLS`] — потому что инструментов больше
//! восьми, а секторов на кольце ровно восемь: геометрия (`menu_ring::SECTORS`)
//! остаётся одной и той же для обеих страниц, меняются только подписи.

/// Что сектор делает при выборе.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Открыть панель интерфейса с таким именем.
    Panel(&'static str),
    /// Показать второе кольцо (подкольцо «Инструменты»).
    Submenu,
    /// Вернуть основное кольцо.
    Back,
    /// Пункт ещё не сделан. Фаза в [`Sector::phase`] — по ней Капри честно
    /// говорит, когда он появится, вместо того чтобы открыть пустое окно.
    Soon,
}

/// Один сектор кольца.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sector {
    /// Идентификатор: он уходит из окна в ядро.
    pub id: &'static str,
    /// Подпись на секторе.
    pub label: &'static str,
    /// Что делать при выборе.
    pub target: Target,
    /// Фаза плана, на которой пункт оживёт (для честной подсказки).
    pub phase: u8,
}

/// Основное кольцо: восемь секторов по часовой стрелке, начиная с верхнего.
pub const SECTORS: [Sector; 8] = [
    Sector { id: "settings", label: "Настройки", target: Target::Panel("settings"), phase: 0 },
    Sector { id: "chat", label: "Чат", target: Target::Panel("chat"), phase: 7 },
    Sector { id: "games", label: "Игры", target: Target::Panel("chess"), phase: 5 },
    Sector { id: "shop", label: "Магазин", target: Target::Panel("shop"), phase: 3 },
    // «Инструменты» — не панель, а второе кольцо: терминал, медиа, перевод и
    // остальное не помещаются в восемь секторов основного.
    Sector { id: "tools", label: "Инструменты", target: Target::Submenu, phase: 4 },
    Sector { id: "notes", label: "Заметки", target: Target::Soon, phase: 8 },
    Sector { id: "levels", label: "Уровни", target: Target::Panel("levels"), phase: 3 },
    Sector { id: "devs", label: "Разработчики", target: Target::Soon, phase: 8 },
];

/// Подкольцо «Инструменты» — тоже восемь секторов, поэтому геометрия та же.
pub const TOOLS: [Sector; 8] = [
    Sector { id: "terminal", label: "Терминал", target: Target::Panel("terminal"), phase: 7 },
    Sector { id: "media", label: "Медиа", target: Target::Panel("media"), phase: 4 },
    Sector { id: "translate", label: "Перевод", target: Target::Panel("translate"), phase: 6 },
    Sector { id: "timer", label: "Таймер", target: Target::Soon, phase: 7 },
    Sector { id: "reminders", label: "Напоминания", target: Target::Soon, phase: 7 },
    Sector { id: "files", label: "Поиск файлов", target: Target::Soon, phase: 8 },
    Sector { id: "shot", label: "Скриншот", target: Target::Soon, phase: 7 },
    Sector { id: "back", label: "Назад", target: Target::Back, phase: 0 },
];

/// Сектор основного кольца по номеру (0…7). Номер вне диапазона заворачивается.
pub fn sector(index: usize) -> &'static Sector {
    &SECTORS[index % SECTORS.len()]
}

/// Сектор по идентификатору — то, что пришло из окна. Ищем по обоим кольцам:
/// окно присылает только идентификатор, а с какой страницы он — не говорит.
pub fn find(id: &str) -> Option<&'static Sector> {
    SECTORS.iter().chain(TOOLS.iter()).find(|s| s.id == id)
}

/// Панель, которую открывает сектор, если он вообще её открывает.
pub fn panel_of(sector: &Sector) -> Option<&'static str> {
    match sector.target {
        Target::Panel(name) => Some(name),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Панели, которые точно есть в `capri-panels`.
    const KNOWN_PANELS: [&str; 8] = [
        "settings", "shop", "levels", "media", "chess", "chat", "terminal", "translate",
    ];

    #[test]
    fn each_ring_has_eight_sectors_with_unique_ids() {
        assert_eq!(SECTORS.len(), 8);
        assert_eq!(TOOLS.len(), 8);
        // Подкольцо рисуется тем же кодом, что и основное кольцо, поэтому
        // число секторов обязано совпадать с `menu_ring::SECTORS`.
        assert_eq!(SECTORS.len(), TOOLS.len());
        for ring in [&SECTORS[..], &TOOLS[..]] {
            let mut ids: Vec<&str> = ring.iter().map(|s| s.id).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), 8, "идентификаторы секторов должны быть разными");
        }
    }

    #[test]
    fn ids_are_unique_across_both_rings() {
        // Окно присылает только идентификатор, а `find` ищет по обоим кольцам:
        // одинаковые идентификаторы на разных страницах было бы не различить.
        let mut ids: Vec<&str> = SECTORS.iter().chain(TOOLS.iter()).map(|s| s.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), SECTORS.len() + TOOLS.len());
    }

    #[test]
    fn ids_and_labels_are_not_empty() {
        for sector in SECTORS.iter().chain(TOOLS.iter()) {
            assert!(!sector.id.is_empty());
            assert!(!sector.label.is_empty());
        }
    }

    #[test]
    fn known_sectors_are_found_by_id() {
        assert_eq!(find("settings").map(|s| s.label), Some("Настройки"));
        assert_eq!(find("shop").map(|s| s.phase), Some(3));
        assert!(find("нет-такого").is_none());
        // Сектор подкольца ищется так же, как сектор основного кольца.
        assert_eq!(find("terminal").map(|s| s.label), Some("Терминал"));
        assert_eq!(find("terminal"), find("terminal"));
    }

    #[test]
    fn index_wraps_around_the_ring() {
        assert_eq!(sector(0).id, "settings");
        assert_eq!(sector(8).id, "settings");
        assert_eq!(sector(9).id, "chat");
    }

    #[test]
    fn every_panel_target_points_to_a_real_panel_name() {
        for sector in SECTORS.iter().chain(TOOLS.iter()) {
            if let Some(panel) = panel_of(sector) {
                assert!(KNOWN_PANELS.contains(&panel), "неизвестная панель: {panel}");
            }
        }
    }

    #[test]
    fn unfinished_sectors_are_honest_about_their_phase() {
        // У сектора, который ещё не сделан, фаза обязана быть объявлена — по ней
        // ядро говорит «появится на фазе N», а не открывает пустое окно.
        let mut soon_count = 0;
        for sector in SECTORS.iter().chain(TOOLS.iter()) {
            if sector.target == Target::Soon {
                soon_count += 1;
                assert!(sector.phase > 0, "у «{}» нет ни панели, ни фазы", sector.label);
            }
        }
        assert!(soon_count > 0, "тест должен что-то проверять");
    }

    #[test]
    fn tools_sector_opens_the_submenu_and_back_returns() {
        assert_eq!(find("tools").map(|s| s.target), Some(Target::Submenu));
        assert_eq!(find("back").map(|s| s.target), Some(Target::Back));
        // Навигационные сектора не панели: панели у них нет.
        assert_eq!(find("tools").and_then(panel_of), None);
        assert_eq!(find("back").and_then(panel_of), None);
    }

    #[test]
    fn terminal_is_reachable_from_the_tools_ring() {
        // Пункт 6 фазы B: панель терминала готова, ей нужен вход из кольца.
        // «Терминал» обязан быть в подкольце и обязан открывать панель.
        let terminal = TOOLS.iter().find(|s| s.id == "terminal").expect("нет сектора «Терминал»");
        assert_eq!(panel_of(terminal), Some("terminal"));
    }

    #[test]
    fn media_moved_from_the_main_ring_to_the_tools_ring() {
        // Раньше «Инструменты» открывали медиа напрямую; теперь медиа — один из
        // инструментов, и на основном кольце его быть не должно.
        assert_eq!(find("media").map(|s| s.label), Some("Медиа"));
        assert!(!SECTORS.iter().any(|s| panel_of(s) == Some("media")));
        assert!(TOOLS.iter().any(|s| panel_of(s) == Some("media")));
    }
}
