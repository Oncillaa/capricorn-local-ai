//! Заметки: «запиши: купить хлеб», «что в заметках», «удали заметку про хлеб».
//!
//! Устроено как [`crate::reminders`]: книга в памяти, файл рядом с
//! `config.json`, разбор человеческой фразы и готовые реплики Капри. Часы
//! читаются только в одном месте, поэтому всё остальное проверяется тестами.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Одна заметка.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct Note {
    /// Номер внутри книги.
    pub id: u64,
    /// Текст заметки.
    pub text: String,
    /// Когда записана (unix-секунды).
    pub at: i64,
}

impl Note {
    /// JSON-вид для клиента: поля заметки плюс человеческое время записи.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "text": self.text,
            "at": self.at,
            "when": human_when(self.at),
        })
    }
}

/// Книга заметок.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct NoteBook {
    /// Все заметки в порядке добавления.
    pub notes: Vec<Note>,
    /// Следующий свободный номер.
    next_id: u64,
}

impl NoteBook {
    /// Пустая книга.
    pub fn new() -> Self {
        Self {
            notes: Vec::new(),
            next_id: 1,
        }
    }

    /// Добавить заметку, вернуть её номер.
    pub fn add(&mut self, text: impl Into<String>, at: i64) -> u64 {
        let max_id = self.notes.iter().map(|n| n.id).max().unwrap_or(0);
        let mut id = if self.next_id == 0 { 1 } else { self.next_id };
        if id <= max_id {
            id = max_id + 1;
        }
        self.next_id = id + 1;
        self.notes.push(Note {
            id,
            text: text.into(),
            at,
        });
        id
    }

    /// Убрать заметку по номеру; `true`, если она нашлась.
    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != id);
        self.notes.len() != before
    }

    /// Убрать все заметки, вернуть число убранных.
    pub fn clear(&mut self) -> usize {
        let removed = self.notes.len();
        self.notes.clear();
        removed
    }

    /// Сколько заметок в книге.
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    /// Пуста ли книга.
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// Заметки, в которых встречается `query` (без учёта регистра).
    pub fn find(&self, query: &str) -> Vec<&Note> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        self.notes
            .iter()
            .filter(|n| n.text.to_lowercase().contains(&needle))
            .collect()
    }

    /// Массив JSON-видов всех заметок.
    pub fn to_json(&self) -> Vec<serde_json::Value> {
        self.notes.iter().map(|n| n.to_json()).collect()
    }
}

/// Хранилище книги в файле `base_dir/notes.json`.
pub struct NoteStore {
    path: PathBuf,
}

impl NoteStore {
    /// Хранилище в каталоге `base_dir`.
    pub fn new(base_dir: &Path) -> Self {
        Self {
            path: base_dir.join("notes.json"),
        }
    }

    /// Путь к файлу книги.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Прочитать книгу; нет файла или битый JSON — пустая книга.
    pub fn load(&self) -> NoteBook {
        let raw = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(_) => return NoteBook::new(),
        };
        let body = raw.trim_start_matches('\u{FEFF}');
        match serde_json::from_str::<NoteBook>(body) {
            Ok(book) => book,
            Err(_) => NoteBook::new(),
        }
    }

    /// Записать книгу целиком.
    pub fn save(&self, book: &NoteBook) -> Result<(), String> {
        let body = serde_json::to_string_pretty(book)
            .map_err(|e| format!("не удалось сериализовать заметки: {e}"))?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("не удалось создать каталог {}: {e}", parent.display()))?;
        }
        std::fs::write(&self.path, body)
            .map_err(|e| format!("не удалось записать {}: {e}", self.path.display()))
    }
}

// ---------------------------------------------------------------------------
// Разбор фразы и речь Капри.
// ---------------------------------------------------------------------------

/// С чего начинается просьба записать. Длинные варианты — первыми, иначе
/// «запиши заметку» срежется как «запиши».
const WRITE_PREFIXES: [&str; 9] = [
    "запиши заметку",
    "создай заметку",
    "добавь заметку",
    "новая заметка",
    "заметка",
    "запиши",
    "запомни",
    "сохрани",
    "напомни записать",
];

/// Служебные слова, которые могут стоять сразу после просьбы.
const FILLERS: [&str; 3] = ["что", "мне", "пожалуйста"];

/// Что человек просит записать — остаток фразы после «запиши…».
///
/// `None` — это не просьба о заметке (тогда фразу разбирает движок или
/// нейросеть). Регистр сохраняется: в заметке могут быть имена.
pub fn requested(command: &str) -> Option<String> {
    let text = command.trim();
    let low = text.to_lowercase();
    let prefix = WRITE_PREFIXES.iter().find(|p| starts_with_word(&low, p))?;
    let skip = prefix.chars().count();
    let rest: String = text.chars().skip(skip).collect();
    Some(trim_edge_fillers(&rest))
}

/// Начинается ли фраза с этого слова (а не с более длинного с тем же началом).
///
/// Иначе «запиши» поймало бы «запишись на приём», а «заметка» — «заметках».
fn starts_with_word(low: &str, prefix: &str) -> bool {
    let Some(rest) = low.strip_prefix(prefix) else {
        return false;
    };
    match rest.chars().next() {
        None => true,
        Some(c) => c.is_whitespace() || c == ',' || c == ':' || c == '.' || c == '!' || c == '-',
    }
}

/// Убрать служебные слова по краям: «, что, …».
fn trim_edge_fillers(text: &str) -> String {
    fn cut(value: &str) -> String {
        value
            .trim()
            .trim_matches(|c: char| matches!(c, ',' | ':' | '.' | '!' | '-' | '—' | '–'))
            .trim()
            .to_string()
    }

    let mut body = cut(text);
    loop {
        let low = body.to_lowercase();
        let found = FILLERS.iter().find(|word| starts_with_word(&low, word));
        let Some(word) = found else { break };
        let skip = word.chars().count();
        let rest: String = body.chars().skip(skip).collect();
        let next = cut(&rest);
        if next.is_empty() {
            body = next;
            break;
        }
        body = next;
    }
    body
}

/// Не просьба записать, а вопрос про заметки?
///
/// «что в заметках», «покажи заметки», «какие у меня заметки».
pub fn asks_about_notes(low: &str) -> bool {
    if !low.contains("заметк") {
        return false;
    }
    ["что", "какие", "покажи", "список", "прочитай", "вспомни", "найди"]
        .iter()
        .any(|w| words(low).any(|x| x.trim_matches(|c: char| !c.is_alphanumeric()) == *w))
}

/// Слова фразы — чтобы предлог «о» не находился внутри слова «что».
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split_whitespace()
}

/// Что искать в заметках: хвост после «про», «о», «об», «где».
///
/// Смотрим только на отдельные слова: искать подстроку нельзя — «о» найдётся
/// внутри «что», и «что в заметках» превратилось бы в запрос «в заметках».
/// Маркеры пропускаются подряд: в «удали заметку про хлеб» их два.
pub fn search_query(low: &str) -> Option<String> {
    let mut after = false;
    for word in words(low) {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric());
        if clean.is_empty() {
            continue;
        }
        let marker = matches!(
            clean,
            "про" | "о" | "об" | "обо" | "где" | "заметку" | "заметка" | "заметке" | "заметках"
        );
        if marker {
            after = true;
            continue;
        }
        if after {
            return Some(clean.to_string());
        }
    }
    None
}

/// Просьба убрать заметку? `Some(None)` — убрать все, `Some(Some(что))` —
/// убрать те, где встречается «что».
pub fn asked_to_remove(low: &str) -> Option<Option<String>> {
    if !low.contains("заметк") {
        return None;
    }
    let remove = ["удали", "убери", "удалить", "убрать", "сотри", "стереть", "забудь"]
        .iter()
        .any(|w| low.contains(w));
    if !remove {
        return None;
    }
    if low.contains("все") || low.contains("всё") {
        return Some(None);
    }
    // «удали заметку про хлеб» / «удали заметку №3».
    Some(search_query(low))
}

/// Подтверждение записи: «Записала: купить хлеб.»
pub fn saved(text: &str) -> String {
    format!("Записала: {}.", text.trim())
}

/// Заметку записывать нечего — спрашиваем.
pub fn ask_what() -> &'static str {
    "Что записать?"
}

/// Перечень заметок.
pub fn list(book: &NoteBook) -> String {
    if book.is_empty() {
        return "Заметок нет.".to_string();
    }
    let parts: Vec<String> = book
        .notes
        .iter()
        .map(|n| format!("{} — {}", n.when_short(), n.text))
        .collect();
    if parts.len() == 1 {
        return format!("Заметка одна: {}.", parts[0]);
    }
    format!("Заметок {}: {}.", parts.len(), parts.join("; "))
}

/// Ответ на поиск по заметкам.
pub fn found(query: &str, hits: &[&Note]) -> String {
    if hits.is_empty() {
        return format!("Про «{query}» в заметках ничего нет.");
    }
    let parts: Vec<String> = hits
        .iter()
        .map(|n| format!("{} — {}", n.when_short(), n.text))
        .collect();
    format!("Нашла {}: {}.", plural(hits.len()), parts.join("; "))
}

/// Ответ на удаление одной заметки.
pub fn removed(text: Option<&str>) -> String {
    match text {
        Some(what) => format!("Убрала заметку: {what}."),
        None => "Такой заметки нет.".to_string(),
    }
}

/// Ответ на «удали все заметки».
pub fn removed_all(count: usize) -> String {
    match count {
        0 => "Заметок и так нет.".to_string(),
        1 => "Убрала единственную заметку.".to_string(),
        many => format!("Убрала все заметки, их было {many}."),
    }
}

/// Форма слова для ответа про поиск.
fn plural(count: usize) -> String {
    let tail = count % 100;
    if (11..=14).contains(&tail) {
        return format!("{count} заметок");
    }
    match count % 10 {
        1 => format!("{count} заметку"),
        2..=4 => format!("{count} заметки"),
        _ => format!("{count} заметок"),
    }
}

/// Когда записана заметка, по-человечески: «сегодня», «вчера», «5 мая».
pub fn human_when(at: i64) -> String {
    let now = now_secs();
    let days = (now - at).max(0) / 86_400;
    match days {
        0 => "сегодня".to_string(),
        1 => "вчера".to_string(),
        2..=6 => format!("{days} дня назад"),
        _ => {
            let date = chrono::DateTime::from_timestamp(at, 0)
                .map(|dt| dt.with_timezone(&chrono::Local))
                .map(|dt| format!("{} {}", dt.format("%-d"), month_name(dt.format("%m").to_string().parse().unwrap_or(1))))
                .unwrap_or_else(|| "давно".to_string());
            date
        }
    }
}

impl Note {
    /// Короткая подпись времени для списка: «сегодня», «вчера», «5 мая».
    fn when_short(&self) -> String {
        human_when(self.at)
    }
}

/// Название месяца по номеру.
fn month_name(month: u32) -> &'static str {
    const NAMES: [&str; 12] = [
        "января", "февраля", "марта", "апреля", "мая", "июня",
        "июля", "августа", "сентября", "октября", "ноября", "декабря",
    ];
    NAMES
        .get(month.saturating_sub(1) as usize)
        .copied()
        .unwrap_or("")
}

/// Текущий момент в unix-секундах — единственное место с системными часами.
pub fn now_secs() -> i64 {
    chrono::Local::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_request_is_cut_from_the_command() {
        assert_eq!(
            requested("запиши купить хлеб").as_deref(),
            Some("купить хлеб")
        );
        assert_eq!(
            requested("запиши заметку: позвонить в сервис").as_deref(),
            Some("позвонить в сервис")
        );
        assert_eq!(
            requested("Запомни, что я люблю кофе").as_deref(),
            Some("я люблю кофе")
        );
        // Регистр сохраняется — в заметке может быть имя.
        assert_eq!(
            requested("запиши позвонить Маше").as_deref(),
            Some("позвонить Маше")
        );
    }

    #[test]
    fn other_commands_are_not_notes() {
        assert!(requested("переведи текст").is_none());
        assert!(requested("открой настройки").is_none());
        // «запишись на приём» — не заметка: это другое слово.
        assert!(requested("запишись на приём к врачу").is_none());
        assert!(requested("").is_none());
        // Само «запиши» без текста — просьба, но пустая.
        assert_eq!(requested("запиши").as_deref(), Some(""));
    }

    #[test]
    fn questions_and_removals_are_recognised() {
        assert!(asks_about_notes("что в заметках"));
        assert!(asks_about_notes("покажи заметки"));
        assert!(!asks_about_notes("запиши купить хлеб"));

        assert_eq!(asked_to_remove("удали все заметки"), Some(None));
        assert_eq!(
            asked_to_remove("удали заметку про хлеб"),
            Some(Some("хлеб".to_string()))
        );
        assert_eq!(asked_to_remove("что в заметках"), None);
    }

    #[test]
    fn search_query_is_taken_by_words_not_by_substring() {
        // «что в заметках» — это список, а не поиск: раньше здесь находилось
        // «о», спрятанное внутри «что», и Капри искала «в заметках».
        assert_eq!(search_query("что в заметках"), None);
        assert_eq!(search_query("покажи заметки"), None);
        assert_eq!(
            search_query("найди в заметках про кофе"),
            Some("кофе".to_string())
        );
        assert_eq!(
            search_query("найди в заметках о поездке"),
            Some("поездке".to_string())
        );
        assert_eq!(
            search_query("удали заметку про хлеб"),
            Some("хлеб".to_string())
        );
        // «о» внутри слова не считается предлогом.
        assert_eq!(search_query("покажи заметки обо мне"), Some("мне".to_string()));
    }

    #[test]
    fn book_add_find_and_remove() {
        let mut book = NoteBook::new();
        assert!(book.is_empty());
        assert_eq!(list(&book), "Заметок нет.");

        let first = book.add("купить хлеб", 1_000_000);
        let second = book.add("позвонить Маше", 1_000_100);
        assert_ne!(first, second);
        assert_eq!(book.len(), 2);

        assert_eq!(book.find("хлеб").len(), 1);
        assert_eq!(book.find("ХЛЕБ").len(), 1);
        assert_eq!(book.find("молоко").len(), 0);
        assert_eq!(book.find("").len(), 0);

        assert!(book.remove(second));
        assert!(!book.remove(999));
        assert_eq!(book.len(), 1);
        assert_eq!(book.clear(), 1);
        assert!(book.is_empty());
        // После очистки номера не повторяются.
        let third = book.add("снова", 2_000_000);
        assert!(third > first);
    }

    #[test]
    fn list_and_search_replies() {
        let mut book = NoteBook::new();
        book.add("купить хлеб", now_secs());
        let one = list(&book);
        assert!(one.starts_with("Заметка одна:"), "{one}");

        book.add("позвонить Маше", now_secs());
        let two = list(&book);
        assert!(two.starts_with("Заметок 2:"), "{two}");

        let hits = book.find("хлеб");
        assert_eq!(found("хлеб", &hits), format!("Нашла 1 заметку: сегодня — купить хлеб."));
        assert_eq!(found("молоко", &book.find("молоко")), "Про «молоко» в заметках ничего нет.");
    }

    #[test]
    fn removal_replies() {
        assert_eq!(removed(None), "Такой заметки нет.");
        assert_eq!(removed(Some("купить хлеб")), "Убрала заметку: купить хлеб.");
        assert_eq!(removed_all(0), "Заметок и так нет.");
        assert_eq!(removed_all(1), "Убрала единственную заметку.");
        assert_eq!(removed_all(4), "Убрала все заметки, их было 4.");
    }

    #[test]
    fn saving_replies() {
        assert_eq!(saved(" купить хлеб "), "Записала: купить хлеб.");
        assert!(ask_what().contains("Что записать"));
    }

    #[test]
    fn human_when_speaks_in_days() {
        let now = now_secs();
        assert_eq!(human_when(now), "сегодня");
        assert_eq!(human_when(now - 86_400), "вчера");
        assert_eq!(human_when(now - 3 * 86_400), "3 дня назад");
        // Давняя дата — месяц словом, а не числом.
        let old = human_when(now - 40 * 86_400);
        assert!(
            ["января", "февраля", "марта", "апреля", "мая", "июня", "июля",
             "августа", "сентября", "октября", "ноября", "декабря"]
                .iter()
                .any(|m| old.ends_with(m)),
            "не та подпись: {old}"
        );
    }

    #[test]
    fn store_roundtrip_and_missing_file() {
        // Пишем в target: системный %TEMP% из рабочей папки закрыт политикой.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("test-tmp")
            .join(format!("notes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = NoteStore::new(&dir);

        // Файла нет — пустая книга, а не паника.
        assert!(store.load().is_empty());

        let mut book = store.load();
        let id = book.add("купить хлеб", 1_700_000_000);
        store.save(&book).expect("запись");

        let back = store.load();
        assert_eq!(back.len(), 1);
        assert_eq!(back.notes[0].id, id);
        assert_eq!(back.notes[0].text, "купить хлеб");
        assert_eq!(back.to_json().len(), 1);

        // Битая книга читается как пустая, а не роняет Капри.
        std::fs::write(store.path(), "{ это не json").expect("испортить");
        assert!(store.load().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
