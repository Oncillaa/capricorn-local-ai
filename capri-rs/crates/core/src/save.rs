//! Игровой прогресс (ECS, опыт, уровни, настроение, скины) — `save_data.json`.
//!
//! В Python-версии этот файл писался относительно текущего каталога и терялся,
//! если Капри запускали из другого места. Здесь путь всегда абсолютный.
//!
//! Экономика простая и честная: за дела (команды, клики, игры) начисляются ECS
//! и опыт, за уровень — награда. Настроение живёт своей жизнью: усталость
//! растёт со временем, радость падает, если Капри долго не трогают, и
//! поднимается от внимания.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Сколько ECS дают за новый уровень.
pub const LEVEL_REWARD_MONEY: i64 = 70;
/// Что кладут в инвентарь за уровень.
pub const SNACK: &str = "вкусняшка";

/// Скин персонажа: покупка меняет палитру рендера.
#[derive(Debug)]
pub struct Skin {
    pub id: &'static str,
    pub name: &'static str,
    pub price: i64,
    /// `None` — исходный вид, без перекраски.
    pub color: Option<[u8; 3]>,
    pub note: &'static str,
}

/// Витрина магазина: от исходного вида к самым дорогим.
pub const SKINS: &[Skin] = &[
    Skin {
        id: "purple",
        name: "Фиолетовый",
        price: 0,
        color: None,
        note: "исходный вид, без перекраски",
    },
    Skin {
        id: "lada",
        name: "Лада",
        price: 120,
        color: Some([255, 34, 96]),
        note: "розовый неон из бара «Лада»",
    },
    Skin {
        id: "shadow",
        name: "Тень",
        price: 180,
        color: Some([120, 120, 150]),
        note: "серая, почти незаметная",
    },
    Skin {
        id: "toxic",
        name: "Токсин",
        price: 240,
        color: Some([200, 255, 40]),
        note: "ядовито-зелёная",
    },
    Skin {
        id: "sunset",
        name: "Закат",
        price: 320,
        color: Some([255, 140, 0]),
        note: "оранжевый закат Найт-Сити",
    },
    Skin {
        id: "neon",
        name: "Неон",
        price: 400,
        color: Some([40, 255, 150]),
        note: "кислотная подсветка",
    },
    Skin {
        id: "ice",
        name: "Лёд",
        price: 500,
        color: Some([100, 200, 255]),
        note: "холодная синь",
    },
];

/// Найти скин по идентификатору.
pub fn skin_by_id(id: &str) -> Option<&'static Skin> {
    SKINS.iter().find(|skin| skin.id == id)
}

/// Цвет скина (название → RGB). `purple` — исходный вид, без перекраски.
///
/// Старое имя `red` из первых версий понимается как «Лада».
pub fn skin_color(name: &str) -> Option<[u8; 3]> {
    let id = if name == "red" { "lada" } else { name };
    skin_by_id(id).and_then(|skin| skin.color)
}

/// Чем закончилась партия в игре.
///
/// Отдельный тип, а не `bool`: ничья — не поражение, и в статистике
/// (и в наградах) она должна считаться иначе.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameOutcome {
    Win,
    Draw,
    Loss,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SaveData {    pub money: i64,
    pub level: u32,
    pub xp: u32,
    pub xp_to_next: u32,
    pub mood_joy: u32,
    pub mood_tired: u32,
    pub equipped_skin: String,
    pub owned_skins: Vec<String>,
    /// «Вкусняшки» и прочие подарки за уровни.
    pub inventory: Vec<String>,
    // Статистика на экране «Уровни».
    pub commands: u64,
    pub pettings: u64,
    pub games: u64,
    pub wins: u64,
    /// Ничьи отдельно: без них «поражения» считаются как «всё остальное»,
    /// и после ничьей счёт врал бы в пользу Капри.
    pub draws: u64,
    pub snacks: u64,
    /// Сколько уровней взято за всё время (для статистики).
    pub levels_taken: u64,
    /// Минуты «жизни» с последнего тика настроения.
    pub mood_minutes: u32,
}

impl Default for SaveData {
    fn default() -> Self {
        Self {
            money: 328,
            level: 4,
            xp: 98,
            xp_to_next: 180,
            mood_joy: 92,
            mood_tired: 46,
            equipped_skin: "purple".into(),
            owned_skins: vec!["purple".into()],
            inventory: Vec::new(),
            commands: 0,
            pettings: 0,
            games: 0,
            wins: 0,
            draws: 0,
            snacks: 0,
            levels_taken: 0,
            mood_minutes: 0,
        }
    }
}

impl SaveData {
    /// Есть ли скин в гардеробе.
    pub fn owns(&self, id: &str) -> bool {
        self.owned_skins.iter().any(|owned| owned == id)
    }

    /// Заполненность полосы опыта 0…1.
    pub fn xp_share(&self) -> f32 {
        if self.xp_to_next == 0 {
            return 0.0;
        }
        (self.xp as f32 / self.xp_to_next as f32).clamp(0.0, 1.0)
    }

    /// Подпись к шкале радости — как на скриншоте «Уровни».
    pub fn joy_label(&self) -> &'static str {
        match self.mood_joy {
            90..=100 => "в восторге",
            70..=89 => "довольная",
            50..=69 => "спокойная",
            30..=49 => "киснет",
            15..=29 => "обиделась",
            _ => "не в духе",
        }
    }

    /// Подпись к шкале усталости.
    pub fn tired_label(&self) -> &'static str {
        match self.mood_tired {
            0..=50 => "бодрая",
            51..=70 => "подсела",
            71..=85 => "устала",
            _ => "вырубается",
        }
    }

    /// Купить и сразу надеть. Возвращает купленный скин.
    pub fn buy_and_equip(&mut self, id: &str) -> Result<&'static Skin, String> {
        let skin = skin_by_id(id).ok_or_else(|| format!("нет такого скина: {id}"))?;
        if self.owns(skin.id) {
            self.equipped_skin = skin.id.to_string();
            return Ok(skin);
        }
        if self.money < skin.price {
            return Err(format!(
                "не хватает {} ECS — у меня {}",
                skin.price - self.money,
                self.money
            ));
        }
        self.money -= skin.price;
        self.owned_skins.push(skin.id.to_string());
        self.equipped_skin = skin.id.to_string();
        Ok(skin)
    }

    /// Надеть уже купленный скин.
    pub fn equip(&mut self, id: &str) -> Result<&'static Skin, String> {
        let skin = skin_by_id(id).ok_or_else(|| format!("нет такого скина: {id}"))?;
        if !self.owns(skin.id) {
            return Err(format!("«{}» ещё не куплен", skin.name));
        }
        self.equipped_skin = skin.id.to_string();
        Ok(skin)
    }

    /// Минутный тик настроения. `night` — сейчас глубокая ночь.
    ///
    /// Возвращает `true`, если что-то изменилось (тогда файл стоит переписать).
    pub fn tick_mood(&mut self, night: bool) -> bool {
        let mut changed = false;
        self.mood_minutes = self.mood_minutes.saturating_add(1);

        // Усталость растёт всегда, ночью — вдвое быстрее.
        let wear = if night { 2 } else { 1 };
        let tired = (self.mood_tired + wear).min(100);
        if tired != self.mood_tired {
            self.mood_tired = tired;
            changed = true;
        }

        // Радость: тает от усталости и просто от долгого одиночества.
        let lonely = if night {
            self.mood_minutes % 2 == 0
        } else {
            self.mood_minutes % 4 == 0
        };
        if (self.mood_tired >= 70 || lonely) && self.mood_joy > 0 {
            self.mood_joy -= 1;
            changed = true;
        }
        changed
    }

    /// Начислить опыт; возвращает, сколько уровней взяли.
    fn add_xp(&mut self, amount: u32) -> u32 {
        self.xp = self.xp.saturating_add(amount);
        let mut gained = 0u32;
        while self.xp >= self.xp_to_next && self.xp_to_next > 0 {
            self.xp -= self.xp_to_next;
            self.level += 1;
            self.xp_to_next += 50;
            gained += 1;
        }
        if gained > 0 {
            self.levels_taken = self.levels_taken.saturating_add(gained as u64);
            self.money += LEVEL_REWARD_MONEY * gained as i64;
            for _ in 0..gained {
                self.inventory.push(SNACK.to_string());
            }
        }
        gained
    }

    /// Выполненная команда: опыт, ECS, немного радости и усталости.
    pub fn on_command(&mut self) -> u32 {
        self.commands += 1;
        self.money += 5;
        self.mood_joy = (self.mood_joy + 1).min(100);
        self.mood_tired = (self.mood_tired + 1).min(100);
        self.add_xp(10)
    }

    /// Погладили (клик по фигуре).
    pub fn on_pet(&mut self) {
        self.pettings += 1;
        self.mood_joy = (self.mood_joy + 3).min(100);
        self.mood_tired = self.mood_tired.saturating_sub(1);
    }

    /// Сыграли партию. `outcome` — чем всё кончилось.
    ///
    /// Ничья считается отдельно: раньше на входе был только `won`, и ничью
    /// приходилось записывать как поражение — счёт в шахматах из-за этого врал.
    pub fn on_game(&mut self, outcome: GameOutcome) -> u32 {
        self.games += 1;
        match outcome {
            GameOutcome::Win => {
                self.wins += 1;
                self.money += 20;
                self.mood_joy = (self.mood_joy + 4).min(100);
                self.add_xp(25)
            }
            GameOutcome::Draw => {
                self.draws += 1;
                self.money += 5;
                self.add_xp(12)
            }
            GameOutcome::Loss => {
                self.mood_joy = self.mood_joy.saturating_sub(2);
                self.add_xp(5)
            }
        }
    }

    /// Съесть «вкусняшку» из инвентаря.
    pub fn eat_snack(&mut self) -> Result<(), String> {
        let index = self
            .inventory
            .iter()
            .position(|item| item == SNACK)
            .ok_or_else(|| "вкусняшек нет — заработай уровень".to_string())?;
        self.inventory.remove(index);
        self.snacks += 1;
        self.mood_joy = (self.mood_joy + 6).min(100);
        self.mood_tired = self.mood_tired.saturating_sub(4);
        Ok(())
    }

    /// Сколько «вкусняшек» лежит в инвентаре.
    pub fn snack_count(&self) -> usize {
        self.inventory.iter().filter(|item| *item == SNACK).count()
    }
}

pub struct SaveManager {
    path: PathBuf,
}

impl SaveManager {
    pub fn new(base_dir: impl AsRef<Path>) -> Self {
        Self { path: base_dir.as_ref().join("save_data.json") }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Прочитать прогресс. Битый/отсутствующий файл — значения по умолчанию.
    pub fn load(&self) -> SaveData {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                log::warn!("save_data.json битый ({e}), беру значения по умолчанию");
                SaveData::default()
            }),
            Err(_) => {
                let data = SaveData::default();
                let _ = self.save(&data);
                data
            }
        }
    }

    pub fn save(&self, data: &SaveData) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let text = serde_json::to_string_pretty(data).unwrap_or_else(|_| "{}".into());
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &self.path)
    }

    /// Изменить прогресс и сразу сохранить. Возвращает новое состояние.
    pub fn update<F: FnOnce(&mut SaveData)>(&self, change: F) -> SaveData {
        let mut data = self.load();
        let level_before = data.level;
        change(&mut data);
        if data.level > level_before {
            log::info!("Капри повысила уровень! Теперь {}", data.level);
        }
        if let Err(e) = self.save(&data) {
            log::error!("Не удалось сохранить прогресс: {e}");
        }
        data
    }

    /// Награда за выполненную команду: +5 монет, +10 опыта,
    /// при переполнении — новый уровень.
    pub fn reward(&self) -> SaveData {
        self.update(|data| {
            data.on_command();
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Свой каталог на каждый тест: иначе параллельные тесты делят один
    /// `save_data.json` и мешают друг другу.
    ///
    /// Живёт в `target/`, а не в системном `Temp`: в некоторых окружениях
    /// (песочницы, урезанный профиль) запись в `%TEMP%` запрещена, и тест
    /// падал бы на `create_dir_all` вместо проверки логики.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("test-tmp")
            .join(format!("{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|e| panic!("каталог {} не создался: {e}", dir.display()));
        dir
    }

    #[test]
    fn a_draw_is_not_a_loss() {
        let mut save = SaveData::default();
        let money = save.money;
        let joy = save.mood_joy;
        save.on_game(GameOutcome::Draw);
        // Ничья не портит настроение и не идёт в «победы», но и не
        // притворяется поражением — раньше именно так и было.
        assert_eq!(save.games, 1);
        assert_eq!(save.wins, 0);
        assert_eq!(save.draws, 1);
        assert!(save.mood_joy >= joy);
        assert!(save.money > money, "за ничью что-то да должно капать");
    }

    #[test]
    fn a_loss_does_not_touch_the_draw_counter() {
        let mut save = SaveData::default();
        save.on_game(GameOutcome::Loss);
        assert_eq!((save.wins, save.draws, save.games), (0, 0, 1));
    }

    #[test]
    fn wins_losses_and_draws_add_up() {
        let mut save = SaveData::default();
        save.on_game(GameOutcome::Win);
        save.on_game(GameOutcome::Loss);
        save.on_game(GameOutcome::Draw);
        // Счёт на странице считается как games - wins - draws, поэтому
        // сумма обязана сходиться, иначе «поражений» покажется на одно больше.
        assert_eq!(save.games, save.wins + save.draws + 1);
    }

    #[test]
    fn level_up_rolls_over() {
        let dir = scratch("save");
        let mgr = SaveManager::new(&dir);
        let mut d = SaveData::default();
        d.xp = d.xp_to_next - 5;
        d.level = 4;
        mgr.save(&d).unwrap();
        let after = mgr.reward();
        assert_eq!(after.level, 5);
        assert!(after.xp < after.xp_to_next);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn level_up_gives_reward() {
        let mut data = SaveData::default();
        let money_before = data.money;
        data.xp = data.xp_to_next - 1;
        let gained = data.on_command();
        assert_eq!(gained, 1);
        // +5 за команду и +70 за уровень.
        assert_eq!(data.money, money_before + 5 + LEVEL_REWARD_MONEY);
        assert_eq!(data.snack_count(), 1);
    }

    #[test]
    fn buy_checks_money_and_ownership() {
        let mut data = SaveData::default();
        data.money = 100;
        // 120 не хватает.
        assert!(data.buy_and_equip("lada").is_err());
        data.money = 200;
        let skin = data.buy_and_equip("lada").expect("куплен");
        assert_eq!(skin.name, "Лада");
        assert_eq!(data.money, 80);
        assert!(data.owns("lada"));
        assert_eq!(data.equipped_skin, "lada");
        // Второй раз — уже есть, денег не берём.
        let money = data.money;
        let _ = data.buy_and_equip("lada").expect("надет");
        assert_eq!(data.money, money);
    }

    #[test]
    fn equip_requires_purchase() {
        let mut data = SaveData::default();
        assert!(data.equip("ice").is_err());
        data.owned_skins.push("ice".into());
        assert!(data.equip("ice").is_ok());
        assert_eq!(data.equipped_skin, "ice");
    }

    #[test]
    fn mood_labels_match_the_screen() {
        let mut data = SaveData::default();
        data.mood_joy = 92;
        data.mood_tired = 46;
        assert_eq!(data.joy_label(), "в восторге");
        assert_eq!(data.tired_label(), "бодрая");
    }

    #[test]
    fn mood_stays_in_range() {
        let mut data = SaveData::default();
        data.mood_joy = 0;
        data.mood_tired = 100;
        for _ in 0..500 {
            data.tick_mood(true);
        }
        assert!(data.mood_joy <= 100);
        assert!(data.mood_tired <= 100);
    }

    #[test]
    fn night_makes_tired_sooner() {
        let mut day = SaveData::default();
        let mut night = SaveData::default();
        for _ in 0..10 {
            day.tick_mood(false);
            night.tick_mood(true);
        }
        assert!(night.mood_tired > day.mood_tired);
    }

    #[test]
    fn snack_from_inventory_can_be_eaten() {
        let mut data = SaveData::default();
        assert!(data.eat_snack().is_err());
        data.inventory.push(SNACK.into());
        let joy = data.mood_joy;
        data.eat_snack().expect("съедена");
        assert_eq!(data.snacks, 1);
        assert_eq!(data.snack_count(), 0);
        assert!(data.mood_joy >= joy);
    }

    #[test]
    fn old_file_without_new_fields_loads() {
        let dir = scratch("save-old");
        let path = dir.join("save_data.json");
        std::fs::write(&path, r#"{"money": 219, "level": 4, "equipped_skin": "purple"}"#)
            .unwrap_or_else(|e| panic!("файл {} не записался: {e}", path.display()));
        let data = SaveManager::new(&dir).load();
        assert_eq!(data.money, 219);
        assert_eq!(data.level, 4);
        assert_eq!(data.commands, 0);
        assert_eq!(data.mood_joy, SaveData::default().mood_joy);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn catalog_ids_are_unique() {
        let mut ids: Vec<&str> = SKINS.iter().map(|skin| skin.id).collect();
        let total = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), total);
        assert!(SKINS.iter().all(|skin| skin.price >= 0));
        // Исходный скин всегда доступен бесплатно.
        assert!(SKINS.iter().any(|skin| skin.id == "purple" && skin.price == 0));
    }
}
