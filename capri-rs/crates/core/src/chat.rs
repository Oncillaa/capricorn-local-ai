//! История чата: реплики, которые переживают перезапуск.
//!
//! Хранится в `chat_history.json` рядом с `config.json`. Формат намеренно
//! простой и «прощающий»: если файл побит или его нет, чат начинается с
//! пустой истории, а не падает.
//!
//! Модуль чистый: чтение и запись — через переданный путь, поэтому тесты
//! работают без Windows и без сети.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Сколько последних реплик держим. Больше — файл растёт без пользы, а
/// модели всё равно уходит только хвост разговора.
pub const MAX_MESSAGES: usize = 200;

/// Кто сказал.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Who {
    User,
    Capri,
}

impl Who {
    pub fn as_str(self) -> &'static str {
        match self {
            Who::User => "user",
            Who::Capri => "capri",
        }
    }
}

/// Одна реплика.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub who: Who,
    pub text: String,
    /// Время в формате `ЧЧ:ММ` — как показывать в ленте.
    /// Пусто, если часы недоступны: страница тогда просто не рисует время.
    #[serde(default)]
    pub time: String,
}

impl Message {
    pub fn user(text: impl Into<String>, time: impl Into<String>) -> Self {
        Self { who: Who::User, text: text.into(), time: time.into() }
    }

    pub fn capri(text: impl Into<String>, time: impl Into<String>) -> Self {
        Self { who: Who::Capri, text: text.into(), time: time.into() }
    }

    /// Для JSON страницы: добавляет строковый `who`.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "who": self.who.as_str(),
            "text": self.text,
            "time": self.time,
        })
    }
}

/// Файл истории.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    pub messages: Vec<Message>,
}

impl History {
    /// Пустая история.
    pub fn new() -> Self {
        Self::default()
    }

    /// Добавить реплику, обрезав слишком длинный хвост.
    pub fn push(&mut self, message: Message) {
        self.messages.push(message);
        if self.messages.len() > MAX_MESSAGES {
            // Режем начало: старые реплики интересны меньше свежих.
            let extra = self.messages.len() - MAX_MESSAGES;
            self.messages.drain(0..extra);
        }
    }

    /// Сколько всего реплик (для статуса).
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Хвост разговора для модели: последние `limit` реплик.
    ///
    /// Нужен, чтобы «Мозги» помнили контекст, но не отправляли всю историю
    /// в запросе — это дорого и медленно.
    pub fn tail(&self, limit: usize) -> &[Message] {
        let start = self.messages.len().saturating_sub(limit);
        &self.messages[start..]
    }

    /// Сериализовать для страницы.
    pub fn to_json(&self) -> Vec<serde_json::Value> {
        self.messages.iter().map(Message::to_json).collect()
    }
}

/// Чтение и запись истории на диске.
#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    /// Хранилище рядом с остальными файлами Капри.
    pub fn new(base_dir: &Path) -> Self {
        Self { path: base_dir.join("chat_history.json") }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Прочитать историю. Битый файл — пустая история, а не ошибка:
    /// пользователь не должен остаться без чата из-за одной кавычки.
    pub fn load(&self) -> History {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(_) => return History::new(),
        };
        // Ведущий BOM ломает `serde_json`, а Блокнот его любит добавлять.
        let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
        serde_json::from_str(text).unwrap_or_else(|_| History::new())
    }

    /// Записать историю.
    pub fn save(&self, history: &History) -> Result<(), String> {
        let json = serde_json::to_string_pretty(history)
            .map_err(|e| format!("история не сериализовалась: {e}"))?;
        std::fs::write(&self.path, json)
            .map_err(|e| format!("{} не записался: {e}", self.path.display()))
    }
}

/// Время «ЧЧ:ММ» по локальным часам.
///
/// `chrono` уже в зависимостях ядра — не выдумываем своё смещение пояса.
pub fn now_hhmm() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("test-tmp")
            .join(format!("chat-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|e| panic!("каталог {} не создался: {e}", dir.display()));
        dir
    }

    #[test]
    fn history_survives_a_restart() {
        let dir = scratch("restart");
        let store = Store::new(&dir);

        let mut history = History::new();
        history.push(Message::user("привет", "12:00"));
        history.push(Message::capri("и тебе", "12:01"));
        store.save(&history).unwrap_or_else(|e| panic!("не сохранилось: {e}"));

        // Новый `Store` — как после перезапуска.
        let loaded = Store::new(&dir).load();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.messages[0].text, "привет");
        assert_eq!(loaded.messages[1].who, Who::Capri);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_an_empty_history() {
        let dir = scratch("missing");
        let store = Store::new(&dir);
        let history = store.load();
        assert!(history.is_empty(), "нет файла — значит не о чем говорить");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_file_does_not_kill_the_chat() {
        let dir = scratch("broken");
        let store = Store::new(&dir);
        std::fs::write(store.path(), "{ это не json").unwrap_or_else(|e| panic!("{e}"));
        let history = store.load();
        assert!(history.is_empty(), "битый файл — начинаем заново");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bom_from_notepad_is_tolerated() {
        // Блокнот любит дописывать BOM, а `serde_json` от него падает.
        let dir = scratch("bom");
        let store = Store::new(&dir);
        let json = serde_json::to_string(&History { messages: vec![Message::user("ok", "")] })
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(store.path(), format!("\u{FEFF}{json}"))
            .unwrap_or_else(|e| panic!("{e}"));
        let history = store.load();
        assert_eq!(history.len(), 1, "BOM не должен ломать чтение");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn long_history_is_trimmed_from_the_start() {
        let mut history = History::new();
        for i in 0..MAX_MESSAGES + 20 {
            history.push(Message::user(format!("реплика {i}"), ""));
        }
        assert_eq!(history.len(), MAX_MESSAGES);
        // Обрезано начало, а не конец: последняя реплика должна остаться.
        assert_eq!(history.messages.last().map(|m| m.text.as_str()), Some("реплика 219"));
    }

    #[test]
    fn tail_gives_the_recent_part() {
        let mut history = History::new();
        for i in 0..10 {
            history.push(Message::user(format!("{i}"), ""));
        }
        let tail = history.tail(3);
        assert_eq!(tail.len(), 3);
        assert_eq!(tail[0].text, "7");
        // Хвост длиннее истории — отдаём всю историю, без паники.
        assert_eq!(history.tail(50).len(), 10);
    }

    #[test]
    fn json_for_the_page_has_a_string_who() {
        let message = Message::capri("ответ", "13:37");
        let json = message.to_json();
        assert_eq!(json["who"], "capri");
        assert_eq!(json["text"], "ответ");
        assert_eq!(json["time"], "13:37");
    }

    #[test]
    fn time_has_the_right_shape() {
        let time = now_hhmm();
        // Либо пусто (нет часов), либо ровно «ЧЧ:ММ».
        assert!(time.is_empty() || time.len() == 5, "время: {time}");
        if !time.is_empty() {
            assert!(time.contains(':'), "время: {time}");
        }
    }
}
