//! Напоминания и таймеры: разбор фраз вида «через 10 минут» и «в 18:30»,
//! книга напоминаний и её хранение в JSON.

use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Одно напоминание.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct Reminder {
    /// Идентификатор внутри книги.
    pub id: u64,
    /// Текст напоминания.
    pub text: String,
    /// Момент срабатывания (unix-секунды).
    pub due: i64,
    /// Уже сработало.
    pub fired: bool,
}

impl Reminder {
    /// JSON-вид для клиента: поля напоминания плюс человеческий остаток времени.
    pub fn to_json(&self, now: i64) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "text": self.text,
            "due": self.due,
            "fired": self.fired,
            "left": human_left(self.due, now),
        })
    }
}

/// Книга напоминаний.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct Book {
    /// Все напоминания в порядке добавления.
    pub reminders: Vec<Reminder>,
    /// Следующий свободный идентификатор.
    next_id: u64,
}

impl Book {
    /// Пустая книга.
    pub fn new() -> Self {
        Self {
            reminders: Vec::new(),
            next_id: 1,
        }
    }

    /// Добавить напоминание, вернуть его id (id не повторяются, начиная с 1).
    pub fn add(&mut self, text: impl Into<String>, due: i64) -> u64 {
        let max_id = self.reminders.iter().map(|r| r.id).max().map_or(0, |v| v);
        let mut id = if self.next_id == 0 { 1 } else { self.next_id };
        if id <= max_id {
            id = max_id + 1;
        }
        self.next_id = id + 1;
        self.reminders.push(Reminder {
            id,
            text: text.into(),
            due,
            fired: false,
        });
        id
    }

    /// Убрать напоминание по id; `true`, если оно нашлось.
    pub fn cancel(&mut self, id: u64) -> bool {
        let before = self.reminders.len();
        self.reminders.retain(|r| r.id != id);
        self.reminders.len() != before
    }

    /// Убрать все напоминания, вернуть число убранных.
    pub fn clear(&mut self) -> usize {
        let removed = self.reminders.len();
        self.reminders.clear();
        removed
    }

    /// Сколько напоминаний в книге.
    pub fn len(&self) -> usize {
        self.reminders.len()
    }

    /// Пуста ли книга.
    pub fn is_empty(&self) -> bool {
        self.reminders.is_empty()
    }

    /// Ещё не сработавшие напоминания.
    pub fn pending(&self) -> Vec<&Reminder> {
        self.reminders.iter().filter(|r| !r.fired).collect()
    }

    /// Забрать сработавшие (`due <= now`), пометив их как `fired`.
    pub fn take_fired(&mut self, now: i64) -> Vec<Reminder> {
        let mut taken = Vec::new();
        for reminder in self.reminders.iter_mut() {
            if !reminder.fired && reminder.due <= now {
                reminder.fired = true;
                taken.push(reminder.clone());
            }
        }
        taken
    }

    /// Массив JSON-видов всех напоминаний.
    pub fn to_json(&self, now: i64) -> Vec<serde_json::Value> {
        self.reminders.iter().map(|r| r.to_json(now)).collect()
    }
}

/// Хранилище книги в файле `base_dir/reminders.json`.
pub struct Store {
    path: PathBuf,
}

impl Store {
    /// Хранилище в каталоге `base_dir`.
    pub fn new(base_dir: &Path) -> Self {
        Self {
            path: base_dir.join("reminders.json"),
        }
    }

    /// Путь к файлу книги.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Прочитать книгу; нет файла или битый JSON — пустая книга.
    pub fn load(&self) -> Book {
        let raw = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(_) => return Book::new(),
        };
        let body = raw.trim_start_matches('\u{FEFF}');
        match serde_json::from_str::<Book>(body) {
            Ok(book) => book,
            Err(_) => Book::new(),
        }
    }

    /// Записать книгу целиком.
    pub fn save(&self, book: &Book) -> Result<(), String> {
        let body = serde_json::to_string_pretty(book)
            .map_err(|e| format!("не удалось сериализовать напоминания: {e}"))?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("не удалось создать каталог {}: {e}", parent.display()))?;
        }
        std::fs::write(&self.path, body)
            .map_err(|e| format!("не удалось записать {}: {e}", self.path.display()))
    }
}

/// Разобрать фразу: вернуть `(текст напоминания, момент срабатывания)`.
///
/// Понимает «через N секунд/минут/часов», «через полчаса», «через час»,
/// «через полтора часа», «через полторы минуты», «в HH:MM», «завтра в HH:MM»
/// и «в HH:MM утра/дня/вечера/ночи». Системные часы не читает — время в `now`.
pub fn parse_when(text: &str, now: i64) -> Option<(String, i64)> {
    let raw: Vec<&str> = text.split_whitespace().collect();
    if raw.is_empty() {
        return None;
    }
    let words: Vec<String> = raw.iter().map(|w| norm_word(w)).collect();
    let mut drop = vec![false; words.len()];

    // «завтра»
    let mut tomorrow = false;
    for (i, word) in words.iter().enumerate() {
        if word == "завтра" {
            tomorrow = true;
            drop[i] = true;
        }
    }

    // Относительное время: «через ...».
    let mut relative: Option<i64> = None;
    if let Some(ci) = words.iter().position(|w| w == "через") {
        drop[ci] = true;
        match words.get(ci + 1).map(|s| s.as_str()) {
            Some("полчаса") => {
                relative = Some(1800);
                drop[ci + 1] = true;
            }
            Some("час") => {
                relative = Some(3600);
                drop[ci + 1] = true;
            }
            Some("полтора") | Some("полторы") => {
                let unit = words.get(ci + 2).map(|s| s.as_str()).unwrap_or_default();
                if unit.starts_with("час") {
                    relative = Some(5400);
                } else if unit.starts_with("мин") {
                    relative = Some(90);
                }
                if relative.is_some() {
                    drop[ci + 1] = true;
                    drop[ci + 2] = true;
                }
            }
            Some(value) => {
                if let Ok(n) = value.parse::<i64>() {
                    let unit = words.get(ci + 2).map(|s| s.as_str()).unwrap_or_default();
                    let factor = if unit.starts_with("сек") {
                        Some(1)
                    } else if unit.starts_with("мин") {
                        Some(60)
                    } else if unit.starts_with("час") {
                        Some(3600)
                    } else {
                        None
                    };
                    if let Some(k) = factor {
                        if n > 0 {
                            relative = Some(n * k);
                            drop[ci + 1] = true;
                            drop[ci + 2] = true;
                        }
                    }
                }
            }
            None => {}
        }
    }

    // Абсолютное время: «в 18:30», «в 9 вечера», «завтра в 9:00».
    let part = words.iter().position(|w| day_part(w).is_some());
    let mut absolute: Option<(u32, u32)> = None;
    let mut time_at: Option<usize> = None;
    if let Some((i, hour, minute)) = words
        .iter()
        .enumerate()
        .find_map(|(i, w)| parse_hhmm(w).map(|(h, m)| (i, h, m)))
    {
        time_at = Some(i);
        let hour = match part.and_then(|p| day_part(&words[p])) {
            Some(k) => apply_day_part(hour, k),
            None => hour,
        };
        absolute = Some((hour, minute));
    } else if let Some(p) = part {
        if p > 0 {
            if let Ok(hour) = words[p - 1].parse::<u32>() {
                if hour <= 23 {
                    let hour = day_part(&words[p]).map_or(hour, |k| apply_day_part(hour, k));
                    absolute = Some((hour, 0));
                    time_at = Some(p - 1);
                }
            }
        }
    }
    if let Some(i) = time_at {
        drop[i] = true;
        // Предлог перед временем.
        if i > 0 && words[i - 1] == "в" {
            drop[i - 1] = true;
        }
    }
    if let Some(p) = part {
        drop[p] = true;
    }

    // Относительное время важнее абсолютного: «через 2 часа» — явная задержка.
    let due = if let Some(secs) = relative {
        Some(now + secs)
    } else if let Some((hour, minute)) = absolute {
        stamp_for(now, hour, minute, tomorrow)
    } else if tomorrow {
        // «завтра» без времени — утро следующего дня.
        stamp_for(now, 9, 0, true)
    } else {
        None
    };
    let due = due?;

    // Текст напоминания — остаток фразы без куска про время.
    let mut kept: Vec<&str> = Vec::new();
    for (i, word) in raw.iter().enumerate() {
        if !drop[i] {
            kept.push(word);
        }
    }
    while kept.first().map_or(false, |w| is_edge_junk(w)) {
        kept.remove(0);
    }
    while kept.last().map_or(false, |w| is_edge_junk(w)) {
        kept.pop();
    }
    let cleaned = kept.join(" ");
    let cleaned = cleaned.trim();
    let message = if cleaned.is_empty() {
        "напоминание".to_string()
    } else {
        cleaned.to_string()
    };
    Some((message, due))
}

/// Человеческий остаток времени до `due`: «через 5 минут», «просрочено».
pub fn human_left(due: i64, now: i64) -> String {
    if due <= now {
        return "просрочено".to_string();
    }
    let left = due - now;
    // Считать секунды под носом смысла нет — говорим словами.
    if left < 5 {
        return "меньше минуты".to_string();
    }
    if left < 60 {
        return format!("через {} {}", left, plural(left, "секунду", "секунды", "секунд"));
    }
    if left < 3600 {
        let minutes = left / 60;
        return format!(
            "через {} {}",
            minutes,
            plural(minutes, "минуту", "минуты", "минут")
        );
    }
    let hours = left / 3600;
    format!("через {} {}", hours, plural(hours, "час", "часа", "часов"))
}

/// Текущее время в unix-секундах — единственное место с системными часами.
pub fn now_secs() -> i64 {
    Local::now().timestamp()
}

/// Слово в нижнем регистре без обрамляющей пунктуации (двоеточие нужно для «18:30»).
fn norm_word(word: &str) -> String {
    word.to_lowercase()
        .trim_matches(|c: char| !c.is_alphanumeric() && c != ':')
        .to_string()
}

/// Служебное слово по краям фразы, которое надо отрезать.
fn is_edge_junk(word: &str) -> bool {
    let w = norm_word(word);
    w.is_empty() || w == "в" || w == "во" || w == "через"
}

/// Разбор «HH:MM» (0..=23 : 0..=59).
fn parse_hhmm(word: &str) -> Option<(u32, u32)> {
    let (h, m) = word.split_once(':')?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return None;
    }
    let hour: u32 = h.parse().ok()?;
    let minute: u32 = m.parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some((hour, minute))
}

/// Часть суток: 0 — ночь, 1 — утро, 2 — день, 3 — вечер.
fn day_part(word: &str) -> Option<u8> {
    if word.starts_with("ноч") {
        Some(0)
    } else if word.starts_with("утр") {
        Some(1)
    } else if word.starts_with("дн") {
        Some(2)
    } else if word.starts_with("вечер") {
        Some(3)
    } else {
        None
    }
}

/// Перевести час в 24-часовой вид по части суток: «9 вечера» -> 21, «12 ночи» -> 0.
fn apply_day_part(hour: u32, part: u8) -> u32 {
    match part {
        0 => {
            if hour == 12 {
                0
            } else {
                hour
            }
        }
        1 => hour,
        _ => {
            if hour == 12 {
                12
            } else if (1..=11).contains(&hour) {
                hour + 12
            } else {
                hour
            }
        }
    }
}

/// Метка времени для локального «HH:MM»: сегодня, если время ещё впереди, иначе завтра.
fn stamp_for(now: i64, hour: u32, minute: u32, tomorrow: bool) -> Option<i64> {
    let base = DateTime::from_timestamp(now, 0)?.with_timezone(&Local);
    let date = base.date_naive();
    if tomorrow {
        return local_stamp(date.succ_opt()?, hour, minute);
    }
    let today = local_stamp(date, hour, minute)?;
    if today < now {
        local_stamp(date.succ_opt()?, hour, minute)
    } else {
        Some(today)
    }
}

/// Локальная дата и время в unix-секундах (с учётом перевода часов).
fn local_stamp(date: NaiveDate, hour: u32, minute: u32) -> Option<i64> {
    let naive = date.and_hms_opt(hour, minute, 0)?;
    if let Some(dt) = Local.from_local_datetime(&naive).earliest() {
        return Some(dt.timestamp());
    }
    // Несуществующее локальное время (перевод часов вперёд) — берём ближайшее после.
    let shifted = naive.checked_add_signed(Duration::hours(1))?;
    Local
        .from_local_datetime(&shifted)
        .earliest()
        .map(|dt| dt.timestamp())
}

/// Форма слова по числу: 1 минуту / 2 минуты / 5 минут (11-14 — всегда «минут»).
fn plural(n: i64, one: &str, few: &str, many: &str) -> String {
    let last_two = n % 100;
    let last = n % 10;
    let form = if (11..=14).contains(&last_two) {
        many
    } else if last == 1 {
        one
    } else if (2..=4).contains(&last) {
        few
    } else {
        many
    };
    form.to_string()
}

// ---------------------------------------------------------------------------
// Речь Капри вокруг напоминаний.
//
// Фразы живут здесь, рядом с разбором времени, а не в оболочке: так их
// покрывают тесты без окна, микрофона и системных часов.
// ---------------------------------------------------------------------------

/// С чего начинается просьба о напоминании. Длинные варианты — первыми,
/// иначе «поставь напоминание» срежется как «напоминание».
const REQUEST_PREFIXES: [&str; 6] = [
    "поставь напоминание",
    "создай напоминание",
    "напоминание",
    "напомни-ка",
    "напомни мне",
    "напомни",
];

/// Служебные слова в начале просьбы: «напомни мне, пожалуйста, …».
const FILLERS: [&str; 2] = ["мне", "пожалуйста"];

/// Что человек просит напомнить — остаток фразы после «напомни».
///
/// `None` — это не просьба о напоминании (тогда фразу разбирает движок или
/// нейросеть). Регистр сохраняется: в тексте могут быть имена.
pub fn requested(command: &str) -> Option<String> {
    let text = command.trim();
    let low = text.to_lowercase();
    let prefix = REQUEST_PREFIXES.iter().find(|p| low.starts_with(**p))?;
    let skip = prefix.chars().count();
    let rest: String = text.chars().skip(skip).collect();
    Some(trim_fillers(&rest))
}

/// Убрать служебные слова по краям: «, мне, пожалуйста, …».
fn trim_fillers(text: &str) -> String {
    fn cut_edges(value: &str) -> String {
        value
            .trim()
            .trim_matches(|c: char| matches!(c, ',' | ':' | '-' | '—' | '–'))
            .trim()
            .to_string()
    }

    let mut body = cut_edges(text);
    loop {
        let low = body.to_lowercase();
        let found = FILLERS.iter().find(|word| {
            match low.strip_prefix(**word) {
                Some(rest) => match rest.chars().next() {
                    None => true,
                    Some(next) => next.is_whitespace() || next == ',' || next == ':',
                },
                None => false,
            }
        });
        let Some(word) = found else { break };
        let skip = word.chars().count();
        let rest: String = body.chars().skip(skip).collect();
        let next = cut_edges(&rest);
        if next.is_empty() {
            body = next;
            break;
        }
        body = next;
    }
    body
}

/// Подтверждение сразу после просьбы: «Хорошо. Напомню через 10 минут: …».
pub fn confirm(text: &str, due: i64, now: i64) -> String {
    format!("Хорошо. Напомню {}: {}.", human_left(due, now), text.trim())
}

/// Время не назвали — спрашиваем.
pub fn ask_when() -> &'static str {
    "На когда напомнить? Скажи, например: «через десять минут»."
}

/// Время есть, а текста нет — спрашиваем.
pub fn ask_what() -> &'static str {
    "О чём напомнить?"
}

/// Объявление сработавших напоминаний одной фразой.
///
/// Несколько сразу — через «И ещё»: Капри произносит одно предложение, а не
/// обрывает себя на середине вторым.
pub fn announce(fired: &[Reminder]) -> String {
    match fired.len() {
        0 => String::new(),
        1 => format!("Напоминаю: {}.", fired[0].text),
        _ => {
            let parts: Vec<&str> = fired.iter().map(|r| r.text.as_str()).collect();
            format!("Напоминаю: {}.", parts.join(". И ещё: "))
        }
    }
}

/// Перечень ожидающих напоминаний.
pub fn list(book: &Book, now: i64) -> String {
    let pending = book.pending();
    if pending.is_empty() {
        return "Напоминаний нет.".to_string();
    }
    let parts: Vec<String> = pending
        .iter()
        .map(|r| format!("{} — {}", human_left(r.due, now), r.text))
        .collect();
    if parts.len() == 1 {
        return format!("Напоминание одно: {}.", parts[0]);
    }
    format!("Напоминаний {}: {}.", parts.len(), parts.join("; "))
}

/// Ответ на отмену одного напоминания.
pub fn cancelled(text: Option<&str>) -> String {
    match text {
        Some(what) => format!("Убрала: {what}."),
        None => "Напоминаний и так нет.".to_string(),
    }
}

/// Ответ на «отмени все».
pub fn cancelled_all(count: usize) -> String {
    match count {
        0 => "Напоминаний и так нет.".to_string(),
        1 => "Убрала одно напоминание.".to_string(),
        many => format!("Убрала все напоминания, их было {many}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// Локальный момент в unix-секундах (тесты не зависят от пояса машины).
    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
        let date = chrono::NaiveDate::from_ymd_opt(year, month, day).expect("дата");
        let naive = date.and_hms_opt(hour, minute, 0).expect("время");
        match chrono::Local.from_local_datetime(&naive) {
            chrono::LocalResult::Single(dt) => dt.timestamp(),
            chrono::LocalResult::Ambiguous(dt, _) => dt.timestamp(),
            chrono::LocalResult::None => {
                let shifted = naive + chrono::Duration::hours(1);
                chrono::Local
                    .from_local_datetime(&shifted)
                    .earliest()
                    .expect("время")
                    .timestamp()
            }
        }
    }

    /// Разбор обязан пройти, иначе тест падает.
    fn parsed(text: &str, now: i64) -> (String, i64) {
        parse_when(text, now).expect("фраза со временем")
    }

    #[test]
    fn relative_minutes_ten() {
        let now = at(2026, 5, 20, 10, 0);
        let (_, due) = parsed("через 10 минут", now);
        assert_eq!(due, now + 600);
    }

    #[test]
    fn relative_minutes_forms() {
        let now = at(2026, 5, 20, 10, 0);
        assert_eq!(parsed("через 5 минут", now).1, now + 300);
        assert_eq!(parsed("через 1 минуту", now).1, now + 60);
        assert_eq!(parsed("через 2 минуты", now).1, now + 120);
    }

    #[test]
    fn relative_half_hour() {
        let now = at(2026, 5, 20, 10, 0);
        assert_eq!(parsed("через полчаса", now).1, now + 1800);
    }

    #[test]
    fn relative_hour() {
        let now = at(2026, 5, 20, 10, 0);
        assert_eq!(parsed("через час", now).1, now + 3600);
    }

    #[test]
    fn relative_hour_and_half() {
        let now = at(2026, 5, 20, 10, 0);
        assert_eq!(parsed("через полтора часа", now).1, now + 5400);
    }

    #[test]
    fn relative_thirty_seconds() {
        let now = at(2026, 5, 20, 10, 0);
        assert_eq!(parsed("через 30 секунд", now).1, now + 30);
    }

    #[test]
    fn absolute_time_today() {
        let now = at(2026, 5, 20, 10, 0);
        assert_eq!(parsed("в 18:30", now).1, at(2026, 5, 20, 18, 30));
    }

    #[test]
    fn absolute_time_tomorrow_when_passed() {
        let now = at(2026, 5, 20, 19, 0);
        assert_eq!(parsed("в 18:30", now).1, at(2026, 5, 21, 18, 30));
    }

    #[test]
    fn absolute_time_explicit_tomorrow() {
        let now = at(2026, 5, 20, 10, 0);
        assert_eq!(parsed("завтра в 9:00", now).1, at(2026, 5, 21, 9, 0));
    }

    #[test]
    fn absolute_time_evening() {
        let now = at(2026, 5, 20, 10, 0);
        assert_eq!(parsed("в 9 вечера", now).1, at(2026, 5, 20, 21, 0));
    }

    #[test]
    fn absolute_time_midnight() {
        let now = at(2026, 5, 20, 10, 0);
        assert_eq!(parsed("в 12 ночи", now).1, at(2026, 5, 21, 0, 0));
    }

    #[test]
    fn garbage_gives_none() {
        let now = at(2026, 5, 20, 10, 0);
        assert!(parse_when("привет", now).is_none());
        assert!(parse_when("через много минут", now).is_none());
    }

    #[test]
    fn text_is_extracted() {
        let now = at(2026, 5, 20, 10, 0);
        let (text, due) = parsed("напомни позвонить маме через 10 минут", now);
        assert_eq!(text, "напомни позвонить маме");
        assert_eq!(due, now + 600);

        let (text, _) = parsed("купить хлеб в 18:30", now);
        assert_eq!(text, "купить хлеб");
    }

    #[test]
    fn text_default_when_only_time() {
        let now = at(2026, 5, 20, 10, 0);
        assert_eq!(parsed("через 5 минут", now).0, "напоминание");
        assert_eq!(parsed("в 18:30", now).0, "напоминание");
    }

    #[test]
    fn human_left_minutes() {
        let now = at(2026, 5, 20, 10, 0);
        assert_eq!(human_left(now + 60, now), "через 1 минуту");
        assert_eq!(human_left(now + 120, now), "через 2 минуты");
        assert_eq!(human_left(now + 300, now), "через 5 минут");
        assert_eq!(human_left(now, now), "просрочено");
        assert_eq!(human_left(now - 10, now), "просрочено");
    }

    #[test]
    fn human_left_hours_and_seconds() {
        let now = at(2026, 5, 20, 10, 0);
        assert_eq!(human_left(now + 3600, now), "через 1 час");
        assert_eq!(human_left(now + 7200, now), "через 2 часа");
        assert_eq!(human_left(now + 11 * 3600, now), "через 11 часов");
        assert_eq!(human_left(now + 12 * 60, now), "через 12 минут");
        assert_eq!(human_left(now + 30, now), "через 30 секунд");
        assert_eq!(human_left(now + 11, now), "через 11 секунд");
        assert_eq!(human_left(now + 3, now), "меньше минуты");
    }

    #[test]
    fn book_ids_and_cancel() {
        let mut book = Book::new();
        assert!(book.is_empty());
        let a = book.add("раз", 100);
        let b = book.add("два", 200);
        let c = book.add("три", 300);
        assert_eq!((a, b, c), (1, 2, 3));
        assert_eq!(book.len(), 3);
        assert!(book.cancel(b));
        assert_eq!(book.len(), 2);
        assert!(!book.cancel(99));
        assert_eq!(book.pending().len(), 2);
        assert_eq!(book.clear(), 2);
        assert!(book.is_empty());
    }

    #[test]
    fn store_roundtrip_and_lifecycle() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("test-tmp")
            .join(format!("rem-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let file = dir.join("reminders.json");

        let now = at(2026, 5, 20, 10, 0);

        // Сохранение и чтение другим Store (имитация перезапуска).
        let store = Store::new(&dir);
        assert_eq!(store.path(), file.as_path());
        let mut book = Book::new();
        let first = book.add("позвонить маме", now + 60);
        let second = book.add("выпить чай", now + 3600);
        store.save(&book).expect("сохранение");

        let loaded = Store::new(&dir).load();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.reminders[0].id, first);
        assert_eq!(loaded.reminders[0].text, "позвонить маме");
        assert_eq!(loaded.reminders[1].id, second);
        assert!(!loaded.reminders[0].fired);

        // Битый файл и файл с BOM.
        std::fs::write(&file, "{ это не json").expect("битый файл");
        assert!(Store::new(&dir).load().is_empty());
        std::fs::write(&file, "\u{FEFF}{\"reminders\":[],\"next_id\":1}").expect("файл с BOM");
        assert!(Store::new(&dir).load().is_empty());

        // Старый JSON без поля fired и без next_id.
        std::fs::write(
            &file,
            r#"{"reminders":[{"id":7,"text":"старое","due":1}]}"#,
        )
        .expect("старый json");
        let mut old = Store::new(&dir).load();
        assert_eq!(old.len(), 1);
        assert!(!old.reminders[0].fired);
        assert_eq!(old.add("новое", 5), 8);

        // take_fired отдаёт только просроченные и только один раз.
        let mut timed = Book::new();
        let late = timed.add("опаздывает", now - 5);
        let soon = timed.add("ещё ждём", now + 300);
        let taken = timed.take_fired(now);
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].id, late);
        assert!(taken[0].fired);
        assert!(timed.take_fired(now).is_empty());
        assert_eq!(timed.pending().len(), 1);
        assert_eq!(timed.pending()[0].id, soon);
        assert_eq!(timed.to_json(now).len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- речь вокруг напоминаний -------------------------------------------

    #[test]
    fn request_is_cut_from_the_command() {
        assert_eq!(
            requested("напомни через 10 минут выпить воды").as_deref(),
            Some("через 10 минут выпить воды")
        );
        assert_eq!(
            requested("Напомни мне, пожалуйста, позвонить маме").as_deref(),
            Some("позвонить маме")
        );
        assert_eq!(
            requested("поставь напоминание в 18:30 купить хлеб").as_deref(),
            Some("в 18:30 купить хлеб")
        );
        // Регистр в тексте сохраняется — там может быть имя.
        assert_eq!(
            requested("напомни через час позвонить Маше").as_deref(),
            Some("через час позвонить Маше")
        );
    }

    #[test]
    fn other_commands_are_not_reminders() {
        assert!(requested("переведи текст").is_none());
        assert!(requested("открой настройки").is_none());
        // Одно только «напомни» — просьба, но без текста: Капри переспросит.
        assert_eq!(requested("напомни").as_deref(), Some(""));
        assert!(requested("").is_none());
    }

    #[test]
    fn request_leads_to_time_and_text() {
        let now = at(2026, 5, 20, 10, 0);
        let body = requested("напомни через 10 минут выпить воды").expect("просьба");
        let (text, due) = parse_when(&body, now).expect("время");
        assert_eq!(text, "выпить воды");
        assert_eq!(due, now + 600);
        assert_eq!(
            confirm(&text, due, now),
            "Хорошо. Напомню через 10 минут: выпить воды."
        );
    }

    #[test]
    fn announce_joins_several_at_once() {
        let fired = vec![
            Reminder { id: 1, text: "выпить воды".into(), due: 0, fired: true },
            Reminder { id: 2, text: "позвонить маме".into(), due: 0, fired: true },
        ];
        assert_eq!(
            announce(&fired[..1]),
            "Напоминаю: выпить воды."
        );
        assert_eq!(
            announce(&fired),
            "Напоминаю: выпить воды. И ещё: позвонить маме."
        );
        assert_eq!(announce(&[]), "");
    }

    #[test]
    fn list_speaks_about_pending() {
        let now = at(2026, 5, 20, 10, 0);
        let mut book = Book::new();
        assert_eq!(list(&book, now), "Напоминаний нет.");

        book.add("выпить воды", now + 300);
        assert_eq!(list(&book, now), "Напоминание одно: через 5 минут — выпить воды.");

        book.add("позвонить маме", now + 3600);
        assert!(list(&book, now).starts_with("Напоминаний 2: через 5 минут — выпить воды;"));
    }

    #[test]
    fn cancel_and_its_replies() {
        assert_eq!(cancelled(None), "Напоминаний и так нет.");
        assert_eq!(cancelled(Some("выпить воды")), "Убрала: выпить воды.");
        assert_eq!(cancelled_all(0), "Напоминаний и так нет.");
        assert_eq!(cancelled_all(1), "Убрала одно напоминание.");
        assert_eq!(
            cancelled_all(3),
            "Убрала все напоминания, их было 3."
        );
    }

    #[test]
    fn questions_when_something_is_missing() {
        assert!(ask_when().contains("На когда"));
        assert!(ask_what().contains("О чём"));
    }
}
