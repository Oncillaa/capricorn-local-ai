//! Игровой прогресс (деньги, опыт, уровни, скины) — `save_data.json`.
//!
//! В Python-версии этот файл писался относительно текущего каталога и терялся,
//! если Капри запускали из другого места. Здесь путь всегда абсолютный.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SaveData {
    pub money: i64,
    pub level: u32,
    pub xp: u32,
    pub xp_to_next: u32,
    pub mood_joy: u32,
    pub mood_tired: u32,
    pub equipped_skin: String,
    pub owned_skins: Vec<String>,
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
        }
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

    /// Начислить награду за выполненную команду: +5 монет, +10 опыта,
    /// при переполнении — новый уровень.
    pub fn reward(&self) -> SaveData {
        let mut data = self.load();
        data.money += 5;
        data.xp += 10;
        while data.xp >= data.xp_to_next {
            data.xp -= data.xp_to_next;
            data.level += 1;
            data.xp_to_next += 50;
            log::info!("Капри повысила уровень! Теперь {}", data.level);
        }
        if let Err(e) = self.save(&data) {
            log::error!("Не удалось сохранить прогресс: {e}");
        }
        data
    }
}

/// Цвета скинов (название → RGB). `purple` — исходный вид, без перекраски.
pub fn skin_color(name: &str) -> Option<[u8; 3]> {
    match name {
        "neon" => Some([40, 255, 150]),
        "red" => Some([255, 34, 96]),
        "ice" => Some([100, 200, 255]),
        "toxic" => Some([200, 255, 40]),
        "sunset" => Some([255, 140, 0]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_up_rolls_over() {
        let dir = std::env::temp_dir().join("capri-test-save");
        let _ = std::fs::create_dir_all(&dir);
        let mgr = SaveManager::new(&dir);
        let mut d = SaveData::default();
        d.xp = d.xp_to_next - 5;
        d.level = 4;
        mgr.save(&d).unwrap();
        let after = mgr.reward();
        assert_eq!(after.level, 5);
        assert!(after.xp < after.xp_to_next);
        let _ = std::fs::remove_file(mgr.path());
    }
}
