//! Настройки: загрузка/сохранение `config.json`, починка «кракозябр»,
//! значения по умолчанию.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Словарь «название → значение» из настроек (сайты, программы, ответы).
pub type Map = BTreeMap<String, String>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // --- разговор ---
    pub enable_ai: bool,
    pub anthropic_api_key: String,
    pub model: String,
    pub local_ai: bool,
    pub local_model: String,
    pub local_ai_url: String,

    // --- слух ---
    pub wake_words: Vec<String>,
    pub mic_device: Option<String>,
    pub mic_gain: f32,
    pub mic_sensitivity: i32,
    pub use_google_stt: bool,
    pub vosk_model: String,
    pub listen_window_seconds: f32,
    pub followup_seconds: f32,
    pub barge_in: bool,

    // --- голос ---
    pub voice: String,
    pub tts_engine: String,
    pub silero_voice: String,
    pub voice_rate: String,
    pub voice_pitch: String,
    pub voice_shift: f32,
    pub voice_lively: bool,
    pub voice_fx: f32,
    pub voice_volume: u8,

    // --- поведение ---
    pub greet_on_start: bool,
    pub chatter: bool,
    pub chatter_min_minutes: f32,
    pub chatter_max_minutes: f32,
    pub roam_chance: f32,
    pub teleport_on_answer: bool,
    pub hide_after_seconds: f32,
    pub search_when_unsure: bool,
    pub type_press_enter: bool,

    // --- обращение ---
    pub user_name_mode: String,
    pub user_name: String,
    pub user_gender: String,

    // --- браузер ---
    pub browser: String,

    // --- пользовательские списки ---
    pub sites: Map,
    pub apps: Map,
    pub app_aliases: Map,
    pub replies: Map,

    /// Всё, чего нет в структуре (свои поля пользователя) — сохраняем как есть.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,

    /// Служебное: откуда загружен файл (в JSON не пишется).
    #[serde(skip)]
    pub path: PathBuf,

    /// Служебное: текст ошибки чтения config.json (в JSON не пишется).
    #[serde(skip)]
    pub load_warning: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        let mut sites = Map::new();
        for (k, v) in [
            ("ютуб", "https://www.youtube.com"),
            ("youtube", "https://www.youtube.com"),
            ("гугл", "https://www.google.com"),
            ("google", "https://www.google.com"),
            ("яндекс", "https://ya.ru"),
            ("вк", "https://vk.com"),
            ("вконтакте", "https://vk.com"),
            ("почту", "https://mail.google.com"),
            ("почта", "https://mail.google.com"),
            ("гитхаб", "https://github.com"),
            ("твич", "https://www.twitch.tv"),
            ("википедию", "https://ru.wikipedia.org"),
            ("википедия", "https://ru.wikipedia.org"),
            ("переводчик", "https://translate.google.com"),
            ("карты", "https://yandex.ru/maps"),
            ("погоду", "https://yandex.ru/pogoda"),
            ("погода", "https://yandex.ru/pogoda"),
            ("chatgpt", "https://chatgpt.com"),
            ("клод", "https://claude.ai"),
            ("кинопоиск", "https://www.kinopoisk.ru"),
            ("авито", "https://www.avito.ru"),
            ("озон", "https://www.ozon.ru"),
            ("вайлдберриз", "https://www.wildberries.ru"),
            ("дипсик", "https://chat.deepseek.com"),
            ("телеграм", "https://web.telegram.org"),
        ] {
            sites.insert(k.to_string(), v.to_string());
        }

        let mut apps = Map::new();
        for (k, v) in [
            ("блокнот", "notepad"),
            ("калькулятор", "calc"),
            ("проводник", "explorer"),
            ("диспетчер задач", "taskmgr"),
            ("параметры", "ms-settings:"),
            ("настройки", "ms-settings:"),
            ("панель управления", "control"),
            ("командную строку", "cmd"),
            ("командная строка", "cmd"),
            ("терминал", "wt"),
            ("пейнт", "mspaint"),
            ("paint", "mspaint"),
            ("ворд", "winword"),
            ("эксель", "excel"),
            ("браузер", "https://www.google.com"),
            ("настройки звука", "ms-settings:sound"),
            ("настройки экрана", "ms-settings:display"),
            ("корзину", "shell:RecycleBinFolder"),
            ("загрузки", "shell:Downloads"),
        ] {
            apps.insert(k.to_string(), v.to_string());
        }

        let mut aliases = Map::new();
        for (k, v) in [
            ("хром", "chrome"),
            ("гугл хром", "google chrome"),
            ("стим", "steam"),
            ("дискорд", "discord"),
            ("телеграм", "telegram"),
            ("телеграмм", "telegram"),
            ("спотифай", "spotify"),
            ("опера", "opera"),
            ("яндекс браузер", "yandex"),
            ("вс код", "visual studio code"),
            ("визуал студио код", "visual studio code"),
            ("обс", "obs"),
            ("эпик геймс", "epic games"),
            ("ватсап", "whatsapp"),
            ("зум", "zoom"),
            ("фотошоп", "photoshop"),
            ("майнкрафт", "minecraft"),
        ] {
            aliases.insert(k.to_string(), v.to_string());
        }

        Self {
            enable_ai: false,
            anthropic_api_key: String::new(),
            model: "claude-sonnet-4-5".into(),
            local_ai: true,
            local_model: "qwen3:8b".into(),
            local_ai_url: "http://127.0.0.1:11434".into(),

            wake_words: ["капри", "каприз", "капре", "кепри", "капли", "каприк"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            mic_device: None,
            mic_gain: 1.0,
            mic_sensitivity: 100,
            use_google_stt: false,
            vosk_model: "models/vosk-model-small-ru-0.22".into(),
            listen_window_seconds: 8.0,
            followup_seconds: 6.0,
            barge_in: true,

            voice: "ru-RU-SvetlanaNeural".into(),
            tts_engine: "edge".into(),
            silero_voice: "kseniya".into(),
            voice_rate: "+10%".into(),
            voice_pitch: "+15Hz".into(),
            voice_shift: 2.0,
            voice_lively: false,
            voice_fx: 1.8,
            voice_volume: 100,

            greet_on_start: true,
            chatter: true,
            chatter_min_minutes: 15.0,
            chatter_max_minutes: 40.0,
            roam_chance: 0.5,
            teleport_on_answer: true,
            hide_after_seconds: 12.0,
            search_when_unsure: true,
            type_press_enter: true,

            user_name_mode: "none".into(),
            user_name: String::new(),
            user_gender: "male".into(),

            browser: String::new(),

            sites,
            apps,
            app_aliases: aliases,
            replies: Map::new(),

            extra: serde_json::Map::new(),
            path: PathBuf::new(),
            load_warning: None,
        }
    }
}

impl Config {
    /// Прочитать `config.json`. Отсутствующий/пустой файл — создать со значениями
    /// по умолчанию. Битый файл — работать на значениях по умолчанию и записать
    /// предупреждение в `load_warning`.
    pub fn load(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let mut cfg = Self::default();
        cfg.path = path.clone();

        let raw = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let _ = cfg.save();
                return cfg;
            }
            Err(e) => {
                cfg.load_warning = Some(format!(
                    "Не смогла прочитать config.json ({e}), работаю со стандартными настройками."
                ));
                return cfg;
            }
        };

        // BOM + возможные «кракозябры» (UTF-8, прочитанный как cp1251).
        let text = decode_json_bytes(&raw);
        if text.trim().is_empty() {
            let _ = cfg.save();
            return cfg;
        }

        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(serde_json::Value::Object(mut map)) => {
                map = repair_object(map);
                let merged = merge_into_defaults(map);
                match serde_json::from_value::<Config>(merged) {
                    Ok(mut c) => {
                        c.path = path;
                        return c;
                    }
                    Err(e) => {
                        cfg.load_warning = Some(format!(
                            "config.json прочитался, но с неверными типами ({e}). \
                             Работаю со стандартными настройками."
                        ));
                        return cfg;
                    }
                }
            }
            Ok(_) => {
                cfg.load_warning =
                    Some("config.json должен быть объектом { … }, работаю со стандартными настройками.".into());
                return cfg;
            }
            Err(e) => {
                cfg.load_warning = Some(format!(
                    "В config.json ошибка в строке {}, столбец {}: {}. \
                     Работаю со стандартными настройками.",
                    e.line(),
                    e.column(),
                    e
                ));
                return cfg;
            }
        }
    }

    /// Записать настройки на диск (атомарно, через временный файл).
    pub fn save(&self) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self)
            .unwrap_or_else(|_| "{}".to_string());
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text.as_bytes())?;
        std::fs::rename(&tmp, &self.path)
    }

    /// Сохранить одно поле верхнего уровня: `set("chatter", true)`.
    ///
    /// Читает файл заново (чтобы не потерять правки пользователя), меняет ключ
    /// и перезаписывает.
    pub fn set_value(&self, key: &str, value: serde_json::Value) -> Result<(), String> {
        let mut map = self.read_raw_map();
        map.insert(key.to_string(), value);
        self.write_raw_map(map)
    }

    /// Сохранить/удалить запись в словаре (`sites`, `apps`, `replies`).
    pub fn set_entry(&self, section: &str, name: &str, value: Option<&str>) -> Result<(), String> {
        let mut map = self.read_raw_map();
        let entry = map
            .entry(section.to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if !entry.is_object() {
            *entry = serde_json::Value::Object(serde_json::Map::new());
        }
        if let serde_json::Value::Object(dict) = entry {
            match value {
                Some(v) => {
                    dict.insert(name.to_string(), serde_json::Value::String(v.to_string()));
                }
                None => {
                    dict.remove(name);
                }
            }
        }
        self.write_raw_map(map)
    }

    fn read_raw_map(&self) -> serde_json::Map<String, serde_json::Value> {
        match std::fs::read(&self.path) {
            Ok(bytes) => {
                let text = decode_json_bytes(&bytes);
                match serde_json::from_str::<serde_json::Value>(&text) {
                    Ok(serde_json::Value::Object(m)) => repair_object(m),
                    _ => serde_json::Map::new(),
                }
            }
            Err(_) => serde_json::Map::new(),
        }
    }

    fn write_raw_map(&self, map: serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
        let text = serde_json::to_string_pretty(&serde_json::Value::Object(map))
            .map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text.as_bytes()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }

    /// Имя, которым Капри называет пользователя.
    pub fn user_display_name(&self) -> String {
        match self.user_name_mode.as_str() {
            "custom" => self.user_name.trim().to_string(),
            "windows" => std::env::var("USERNAME").unwrap_or_default(),
            "pc" => std::env::var("COMPUTERNAME").unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// Слова-обращения, отсортированные по убыванию длины (длинные — раньше,
    /// чтобы «каприз» не распознался как «капри» + «з»).
    pub fn sorted_wake_words(&self) -> Vec<String> {
        let mut w = self.wake_words.clone();
        w.sort_by_key(|s| std::cmp::Reverse(s.chars().count()));
        w
    }
}

/// Декодировать байты в строку: снимает BOM, чинит «кракозябры».
pub fn decode_json_bytes(raw: &[u8]) -> String {
    let raw = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(raw);
    match std::str::from_utf8(raw) {
        Ok(s) => repair_mojibake(s),
        Err(_) => {
            // Файл не в UTF-8 — значит, его писали в cp1251.
            let (cow, _, _) = encoding_rs::WINDOWS_1251.decode(raw);
            repair_mojibake(cow.as_ref())
        }
    }
}

/// Починить строку вида «РєР°РїСЂРё» → «капри».
///
/// Так выглядит UTF-8, прочитанный как cp1251: байты русских букв попали
/// в диапазон, который cp1251 отображает в «Р», «С», «Ѓ» и т. п.
pub fn repair_mojibake(s: &str) -> String {
    if !looks_like_mojibake(s) {
        return s.to_string();
    }
    let (bytes, _, had_errors) = encoding_rs::WINDOWS_1251.encode(s);
    if had_errors {
        return s.to_string();
    }
    match std::str::from_utf8(&bytes) {
        Ok(fixed) if !fixed.is_empty() => {
            log::info!("Починены кракозябры в настройках: {s:?} → {fixed:?}");
            fixed.to_string()
        }
        _ => s.to_string(),
    }
}

fn looks_like_mojibake(s: &str) -> bool {
    if !s.chars().any(|c| c.is_cyrillic_upper_ish()) {
        return false;
    }
    // Признак: много заглавных «Р»/«С»/«Ѓ»/«‚» подряд без обычных строчных букв.
    let mut suspicious = 0usize;
    let mut total = 0usize;
    for c in s.chars() {
        if c.is_whitespace() || c.is_ascii() {
            continue;
        }
        total += 1;
        if matches!(
            c,
            'Р' | 'С' | 'Ђ' | 'Ѓ' | 'ѓ' | '‚' | '„' | '№' | '¤' | '¶' | 'µ' | 'ё' | '°'
        ) {
            suspicious += 1;
        }
    }
    total > 0 && suspicious * 2 >= total
}

trait CyrIsh {
    fn is_cyrillic_upper_ish(&self) -> bool;
}

impl CyrIsh for char {
    fn is_cyrillic_upper_ish(&self) -> bool {
        matches!(*self, 'Р' | 'С' | 'Ђ' | 'Ѓ' | 'Ћ' | 'Џ')
    }
}

fn repair_object(
    map: serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    for (k, v) in map {
        out.insert(repair_mojibake(&k), repair_value(v));
    }
    out
}

fn repair_value(v: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::String(s) => Value::String(repair_mojibake(&s)),
        Value::Array(items) => Value::Array(items.into_iter().map(repair_value).collect()),
        Value::Object(map) => Value::Object(repair_object(map)),
        other => other,
    }
}

/// Дополнить пользовательский JSON недостающими ключами по умолчанию, чтобы
/// serde не спотыкался (и чтобы новые настройки появлялись сами).
fn merge_into_defaults(
    user: serde_json::Map<String, serde_json::Value>,
) -> serde_json::Value {
    let defaults = serde_json::to_value(Config::default()).unwrap_or_default();
    let serde_json::Value::Object(mut base) = defaults else {
        return serde_json::Value::Object(user);
    };
    for (k, v) in user {
        match (base.get_mut(&k), &v) {
            (Some(serde_json::Value::Object(dst)), serde_json::Value::Object(src)) => {
                for (sk, sv) in src {
                    if sv.is_null() {
                        dst.remove(sk);
                    } else {
                        dst.insert(sk.clone(), sv.clone());
                    }
                }
            }
            _ => {
                base.insert(k, v);
            }
        }
    }
    serde_json::Value::Object(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mojibake_is_repaired() {
        assert_eq!(repair_mojibake("РєР°РїСЂРё"), "капри");
        assert_eq!(repair_mojibake("СЋС‚СѓР±"), "ютуб");
        assert_eq!(repair_mojibake("капри"), "капри");
        assert_eq!(repair_mojibake("youtube"), "youtube");
    }

    #[test]
    fn defaults_roundtrip() {
        let cfg = Config::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.wake_words, cfg.wake_words);
        assert_eq!(back.sites.len(), cfg.sites.len());
    }
}
