//! Перевод текста с экрана.
//!
//! Источник текста — [`crate::ocr`]-модуль в `capri-win` (WinRT), но сам
//! перевод от Windows не зависит: это обычный HTTP-запрос. Поэтому модуль
//! живёт в ядре и умеет работать через любой из провайдеров.
//!
//! Провайдеров три, и выбираются они по тому, что у пользователя настроено:
//!
//! * **локальная модель** (`Ollama`) — по умолчанию, если `local_ai` включён.
//!   Ничего не уходит в сеть, работает без ключей. Качество перевода у
//!   9-миллиардной модели ниже, чем у облака, зато бесплатно и приватно.
//! * **облако** — уже существующий [`crate::ai::AiChat`] (Claude или любой
//!   OpenAI-совместимый), если в настройках есть ключ.
//! * **LibreTranslate** — если указан адрес сервера. Это единственный вариант,
//!   который не «сочиняет», а переводит дословно.
//!
//! Ответы кэшируются на диск: один и тот же текст на экране встречается часто
//! (меню, кнопки, подписи), а каждый запрос — это секунды или деньги.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use crate::ai::{AiChat, LocalAi};
use crate::config::Config;

/// Язык перевода.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Language {
    /// Код для `config.json` и провайдеров: `ru`, `en`, …
    pub code: &'static str,
    /// Название по-русски — для панели.
    pub name: &'static str,
    /// Как язык называется для модели: «русский», «английский».
    pub russian_name: &'static str,
}

/// Языки, между которыми переводим.
///
/// Список короткий намеренно: OCR распознаёт только те языки, для которых в
/// системе стоит пакет, а переводить на суахили из меню с восемью строками
/// никто не станет. Расширять — одна строка.
pub const LANGUAGES: [Language; 10] = [
    Language { code: "ru", name: "Русский", russian_name: "русский" },
    Language { code: "en", name: "Английский", russian_name: "английский" },
    Language { code: "de", name: "Немецкий", russian_name: "немецкий" },
    Language { code: "fr", name: "Французский", russian_name: "французский" },
    Language { code: "es", name: "Испанский", russian_name: "испанский" },
    Language { code: "it", name: "Итальянский", russian_name: "итальянский" },
    Language { code: "uk", name: "Украинский", russian_name: "украинский" },
    Language { code: "tr", name: "Турецкий", russian_name: "турецкий" },
    Language { code: "zh", name: "Китайский", russian_name: "китайский" },
    Language { code: "ja", name: "Японский", russian_name: "японский" },
];

/// Язык по коду. Незнакомый код — русский: переводить «в никуда» бессмысленно.
pub fn language(code: &str) -> &'static Language {
    // Тег может прийти с регионом (`de-DE`) — из настроек, из старого файла
    // или от OCR. Берём только основной код: иначе «de-DE» не нашлось бы, и
    // человек молча получил бы перевод на русский вместо немецкого.
    let code = code
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    // Пустая строка — тоже «незнакомый код»: в старых config.json поля не было
    // вовсе, и serde подставил пустую строку. Без этой ветки панель показывала
    // бы «перевод на » и уходила в сеть с пустым целевым языком.
    if code.is_empty() {
        return &LANGUAGES[0];
    }
    LANGUAGES
        .iter()
        .find(|language| language.code == code)
        .unwrap_or(&LANGUAGES[0])
}

/// Знает ли проект такой язык.
pub fn knows(code: &str) -> bool {
    let code = code.trim().to_ascii_lowercase();
    LANGUAGES.iter().any(|language| language.code == code)
}

/// Терминальный код языка из тега OCR (`en-US` → `en`).
///
/// OCR отдаёт BCP-47, а провайдерам нужен короткий код. Заодно это источник
/// для определения исходного языка: распознали английским — значит текст
/// английский.
pub fn from_tag(tag: &str) -> &'static str {
    let primary = tag
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    LANGUAGES
        .iter()
        .find(|language| language.code == primary)
        .map(|language| language.code)
        .unwrap_or("en")
}

/// Что вернулось из перевода.
#[derive(Debug, Clone, PartialEq)]
pub struct Translated {
    /// Переведённый текст.
    pub text: String,
    /// Откуда перевели (`en`).
    pub from: &'static str,
    /// Куда (`ru`).
    pub to: &'static str,
    /// Кто перевёл — показываем в панели, чтобы «странный перевод» было с чем
    /// соотнести.
    pub provider: &'static str,
    /// Взяли из кэша, а не спросили заново.
    pub cached: bool,
}

/// Чем переводим.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Local,
    Cloud,
    Libre,
}

impl Backend {
    fn label(self) -> &'static str {
        match self {
            Backend::Local => "локальная модель",
            Backend::Cloud => "облако",
            Backend::Libre => "LibreTranslate",
        }
    }
}

/// Переводчик.
pub struct Translator {
    local: LocalAi,
    cloud: AiChat,
    local_enabled: bool,
    libre_url: String,
    cache_dir: PathBuf,
}

/// Потолок текста за один запрос.
///
/// Экран целиком (статья, страница настроек) в один запрос не влезет: у
/// локальной модели окно контекста небольшое, а облачные считают деньги за
/// токены. Длинный текст режем по абзацам и переводим частями.
const CHUNK_CHARS: usize = 1800;

/// Сколько частей переводим за раз. Дальше — отказ с понятным текстом:
/// молча обрезать перевод хуже, чем сказать «выдели меньше».
const MAX_CHUNKS: usize = 8;

impl Translator {
    /// `cache_dir` — куда складывать переводы. Если `None`, кэш выключен.
    pub fn new(cfg: &Config, cache_dir: Option<PathBuf>) -> Self {
        Self {
            local: LocalAi::new(cfg),
            cloud: AiChat::new(cfg),
            local_enabled: cfg.local_ai,
            libre_url: cfg.translate_libretranslate_url.trim_end_matches('/').to_string(),
            cache_dir: cache_dir.unwrap_or_default(),
        }
    }

    /// Кто будет переводить при текущих настройках — для панели.
    pub fn backend(&self) -> Result<Backend, String> {
        if !self.libre_url.is_empty() {
            return Ok(Backend::Libre);
        }
        if self.local_enabled {
            return Ok(Backend::Local);
        }
        if self.cloud.available() {
            return Ok(Backend::Cloud);
        }
        Err(
            "переводить нечем: включи локальный ИИ в настройках, добавь ключ нейросети \
             или укажи адрес LibreTranslate."
                .into(),
        )
    }

    /// Название провайдера для подписи в панели.
    pub fn backend_label(&self) -> String {
        match self.backend() {
            Ok(backend) => backend.label().to_string(),
            Err(_) => "нет провайдера".into(),
        }
    }

    /// Перевести текст.
    ///
    /// `from` — язык исходника (`None` — определить по самому тексту или взять
    /// язык распознавания). `to` — целевой код (`ru`, `en`, …).
    pub fn translate(&self, text: &str, from: Option<&str>, to: &str) -> Result<Translated, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("нечего переводить — выделенный текст пустой".into());
        }
        let to = language(to);

        let chunks = split_chunks(text);
        if chunks.len() > MAX_CHUNKS {
            return Err(format!(
                "слишком много текста: {} частей, а перевожу не больше {MAX_CHUNKS}. \
                 Выдели область поменьше.",
                chunks.len()
            ));
        }

        // Язык исходника: явный, иначе определяем сами.
        let from_code = match from {
            Some(code) if knows(code) && language(code).code != to.code => language(code).code,
            _ => detect_language(text, to.code),
        };

        // Проверяем «перевод в себя» до выбора провайдера: если переводить не с
        // чего, требовать настроенный провайдер было бы издевательством.
        if from_code == to.code {
            return Ok(Translated {
                text: text.to_string(),
                from: from_code,
                to: to.code,
                provider: "не требуется",
                cached: false,
            });
        }

        let backend = self.backend()?;

        let source_name = language(from_code).russian_name;
        let target_name = to.russian_name;

        let mut parts = Vec::with_capacity(chunks.len());
        let mut all_cached = true;
        for chunk in &chunks {
            let key = cache_key(chunk, from_code, to.code, backend);
            if let Some(hit) = self.cache_get(&key) {
                parts.push(hit);
                continue;
            }
            all_cached = false;
            let translated = match backend {
                Backend::Local => self.via_local(chunk, source_name, target_name)?,
                Backend::Cloud => self.via_cloud(chunk, source_name, target_name)?,
                Backend::Libre => self.via_libre(chunk, from_code, to.code)?,
            };
            self.cache_put(&key, &translated);
            parts.push(translated);
        }

        Ok(Translated {
            text: parts.join("\n\n"),
            from: from_code,
            to: to.code,
            provider: backend.label(),
            cached: all_cached,
        })
    }

    /// Локальная модель. Промпт намеренно жёсткий: без него модель добавляет
    /// «Вот перевод:» и объяснения, а панель показывает это как перевод.
    fn via_local(&self, text: &str, from: &str, to: &str) -> Result<String, String> {
        let prompt = format!(
            "Переведи текст с {from} на {to}. Верни только перевод, без пояснений, \
             без кавычек, без слова «перевод». Сохрани переводы строк.\n\n{text}"
        );
        // `ask` внутри добавляет персону Капри — для перевода это лишнее, и
        // `ask_raw` нужен именно чтобы её не было.
        let answer = self
            .local
            .ask_plain(&prompt)
            .ok_or_else(|| {
                "локальная модель не ответила. Проверь, что Ollama запущена и модель скачана."
                    .to_string()
            })?;
        Ok(clean_translation(&answer))
    }

    fn via_cloud(&self, text: &str, from: &str, to: &str) -> Result<String, String> {
        let prompt = format!(
            "Переведи текст с {from} на {to}. Верни только перевод, без пояснений \
             и без кавычек. Сохрани переводы строк.\n\n{text}"
        );
        let answer = self.cloud.ask(&prompt).ok_or_else(|| {
            "нейросеть не ответила: проверь ключ и соединение".to_string()
        })?;
        Ok(clean_translation(&answer))
    }

    fn via_libre(&self, text: &str, from: &str, to: &str) -> Result<String, String> {
        let body = json!({
            "q": text,
            "source": from,
            "target": to,
            "format": "text",
        });
        let mut resp = ureq::post(&format!("{}/translate", self.libre_url))
            .config()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .send_json(&body)
            .map_err(|e| format!("LibreTranslate не ответил: {e}"))?;
        let value: Value = resp
            .body_mut()
            .read_json()
            .map_err(|e| format!("LibreTranslate ответил непонятно: {e}"))?;
        value
            .get("translatedText")
            .and_then(|text| text.as_str())
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
            .ok_or_else(|| "LibreTranslate вернул пустой перевод".to_string())
    }

    fn cache_path(&self, key: &str) -> Option<PathBuf> {
        if self.cache_dir.as_os_str().is_empty() {
            return None;
        }
        Some(self.cache_dir.join(format!("{key}.txt")))
    }

    fn cache_get(&self, key: &str) -> Option<String> {
        let path = self.cache_path(key)?;
        let text = std::fs::read_to_string(path).ok()?;
        let text = text.trim();
        if text.is_empty() {
            None
        } else {
            Some(text.to_string())
        }
    }

    fn cache_put(&self, key: &str, value: &str) {
        let Some(path) = self.cache_path(key) else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Кэш — удобство, а не данные пользователя: если записать не удалось,
        // молчим и просто переводим в следующий раз заново.
        let _ = std::fs::write(path, value);
    }
}

/// Разбить текст на части по абзацам, не разрывая строки посередине.
pub fn split_chunks(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        // Одна строка длиннее лимита — режем её по словам.
        if line.chars().count() > CHUNK_CHARS {
            if !current.trim().is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            for piece in split_long_line(line) {
                chunks.push(piece);
            }
            continue;
        }
        if current.chars().count() + line.chars().count() + 1 > CHUNK_CHARS {
            if !current.trim().is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }
    if !current.trim().is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        chunks.push(text.to_string());
    }
    chunks
}

/// Разрезать слишком длинную строку по словам.
fn split_long_line(line: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    for word in line.split_inclusive(char::is_whitespace) {
        if current.chars().count() + word.chars().count() > CHUNK_CHARS && !current.is_empty() {
            parts.push(std::mem::take(&mut current));
        }
        current.push_str(word);
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    parts
}

/// Убрать обёртки, которые модель добавляет вокруг перевода.
///
/// Даже с жёстким промптом локальные модели любят начать с «Перевод:» или
/// обернуть всё в кавычки. Панель показывает текст как есть, поэтому чистим.
pub fn clean_translation(text: &str) -> String {
    let mut text = text.trim().to_string();

    // «Вот перевод:», «Перевод:», «Translation:» в начале.
    for prefix in [
        "Вот перевод:",
        "Вот перевод",
        "Перевод:",
        "Перевод —",
        "Текст перевода:",
        "Translation:",
        "Here is the translation:",
    ] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.trim().to_string();
            break;
        }
    }

    // Целиком в кавычках — снимаем парные.
    for (open, close) in [('«', '»'), ('"', '"'), ('“', '”')] {
        if text.starts_with(open) && text.ends_with(close) && text.chars().count() > 2 {
            let inner: String = text
                .chars()
                .skip(1)
                .take(text.chars().count() - 2)
                .collect();
            text = inner.trim().to_string();
            break;
        }
    }

    text
}

/// Определить язык текста: кириллица → русский, иероглифы → китайский,
/// катакана/хирагана → японский, иначе английский.
///
/// Это не «настоящее» определение языка, а грубая, но предсказуемая проверка
/// по алфавиту. Для экрана этого достаточно: если распознали кириллицу, а
/// переводим на русский, переводить нечего — и это главное, что нужно знать.
pub fn detect_language(text: &str, target: &str) -> &'static str {
    let mut cyrillic = 0usize;
    let mut latin = 0usize;
    let mut han = 0usize;
    let mut kana = 0usize;
    let mut hiragana_katakana = false;
    for ch in text.chars() {
        match ch as u32 {
            0x0400..=0x04FF => cyrillic += 1,
            0x4E00..=0x9FFF => han += 1,
            0x3040..=0x30FF => {
                kana += 1;
                hiragana_katakana = true;
            }
            _ if ch.is_ascii_alphabetic() => latin += 1,
            _ => {}
        }
    }
    let _ = hiragana_katakana;

    if kana > 0 {
        return "ja";
    }
    if han > cyrillic && han > latin {
        return "zh";
    }
    if cyrillic > latin {
        // Кириллица: либо русский, либо украинский. Различаем по буквам,
        // которых нет в русском.
        let lower = text.to_lowercase();
        if lower.contains('і') || lower.contains('ї') || lower.contains('є') || lower.contains('ґ') {
            if target == "uk" {
                // Уже украинский — переводить не с чего.
                return "uk";
            }
            return "uk";
        }
        return "ru";
    }
    "en"
}

/// Ключ кэша: текст + направление + провайдер.
///
/// `DefaultHasher` даёт одинаковое значение при каждом запуске (ключи внутри
/// фиксированы), поэтому кэш живёт между запусками. Криптостойкость здесь не
/// нужна — это имя файла, а не подпись.
fn cache_key(text: &str, from: &str, to: &str, backend: Backend) -> String {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    from.hash(&mut hasher);
    to.hash(&mut hasher);
    backend.label().hash(&mut hasher);
    format!("{from}-{to}-{:016x}", hasher.finish())
}

/// Папка кэша переводов рядом с программой.
pub fn cache_dir(base: &Path) -> PathBuf {
    base.join("cache").join("translate")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_are_known_by_code() {
        assert_eq!(language("ru").name, "Русский");
        assert_eq!(language(" EN ").code, "en");
        // Незнакомый код — русский, а не паника.
        assert_eq!(language("xx").code, "ru");
        // Пустая строка бывает в config.json, где поля ещё не было.
        assert_eq!(language("").code, "ru");
        assert!(knows("de"));
        assert!(!knows("sw"));
    }

    #[test]
    fn language_tags_with_a_region_still_resolve() {
        // Настройки и старые файлы могут хранить полный тег. Без разбора
        // региона «de-DE» превратился бы в русский — молча и незаметно.
        assert_eq!(language("de-DE").code, "de");
        assert_eq!(language("en_US").code, "en");
        assert_eq!(language("JA-jp").code, "ja");
        // Регион не спасает незнакомый язык — он всё равно русский.
        assert_eq!(language("sw-KE").code, "ru");
    }

    #[test]
    fn ocr_tags_become_short_codes() {
        assert_eq!(from_tag("en-US"), "en");
        assert_eq!(from_tag("ru"), "ru");
        assert_eq!(from_tag("ru-RU"), "ru");
        assert_eq!(from_tag("de-DE"), "de");
        // Незнакомый тег считаем английским: OCR чаще всего видит английский.
        assert_eq!(from_tag("sw-KE"), "en");
        assert_eq!(from_tag(""), "en");
    }

    #[test]
    fn language_is_detected_by_alphabet() {
        assert_eq!(detect_language("Привет, мир", "en"), "ru");
        assert_eq!(detect_language("Hello world", "ru"), "en");
        // Украинский отличается буквами, которых нет в русском.
        assert_eq!(detect_language("Привіт, світе", "ru"), "uk");
        // Катакана/хирагана важнее иероглифов: японский пишется и ими тоже.
        assert_eq!(detect_language("こんにちは世界", "ru"), "ja");
        assert_eq!(detect_language("你好世界", "ru"), "zh");
    }

    #[test]
    fn text_without_letters_falls_back_to_english() {
        // Цифры и знаки: пусть решает провайдер, а не мы.
        assert_eq!(detect_language("123 456 + -", "ru"), "en");
        assert_eq!(detect_language("", "ru"), "en");
    }

    #[test]
    fn translation_wrappers_are_removed() {
        assert_eq!(clean_translation("Перевод: Привет"), "Привет");
        assert_eq!(clean_translation("Вот перевод:\nПривет"), "Привет");
        assert_eq!(clean_translation("«Привет»"), "Привет");
        assert_eq!(clean_translation("\"Hello\""), "Hello");
        assert_eq!(clean_translation("  Привет  "), "Привет");
        // Одинарные кавычки не трогаем: это может быть частью текста.
        assert_eq!(clean_translation("'quoted'"), "'quoted'");
        // Короткая строка из одной кавычки не должна стать пустой.
        assert_eq!(clean_translation("«"), "«");
    }

    #[test]
    fn short_text_stays_in_one_chunk() {
        let chunks = split_chunks("Первая строка\nВторая строка");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Первая строка\nВторая строка");
    }

    #[test]
    fn long_text_is_split_by_lines() {
        // Строки по 100 символов, лимит 1800 → примерно 18 строк на часть.
        let text = (0..60)
            .map(|index| format!("строка номер {index:02} {}", "х".repeat(80)))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = split_chunks(&text);
        assert!(chunks.len() > 1, "длинный текст должен делиться");
        for chunk in &chunks {
            assert!(
                chunk.chars().count() <= CHUNK_CHARS + 120,
                "часть получилась длиннее лимита: {}",
                chunk.chars().count()
            );
        }
        // Ничего не потеряли.
        let total: usize = chunks.iter().map(|c| c.chars().count()).sum();
        assert!(total >= text.chars().count() - chunks.len());
    }

    #[test]
    fn one_huge_line_is_split_by_words() {
        let line = "слово ".repeat(600);
        let chunks = split_chunks(&line);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= CHUNK_CHARS + 10);
        }
    }

    #[test]
    fn empty_text_gives_one_empty_chunk_not_zero() {
        // Панель проверяет пустоту отдельно, но `split_chunks` не должен
        // возвращать пустой список: иначе перевод «ничего» пройдёт молча.
        assert_eq!(split_chunks("").len(), 1);
        assert_eq!(split_chunks("   ").len(), 1);
    }

    #[test]
    fn cache_key_depends_on_direction_and_provider() {
        let a = cache_key("Привет", "ru", "en", Backend::Local);
        let b = cache_key("Привет", "ru", "en", Backend::Local);
        assert_eq!(a, b, "один и тот же запрос должен попадать в один файл");

        let different_direction = cache_key("Привет", "ru", "de", Backend::Local);
        assert_ne!(a, different_direction);

        let different_backend = cache_key("Привет", "ru", "en", Backend::Cloud);
        assert_ne!(a, different_backend);

        let different_text = cache_key("Пока", "ru", "en", Backend::Local);
        assert_ne!(a, different_text);
    }

    #[test]
    fn cache_key_is_a_safe_file_name() {
        let key = cache_key("текст со пробелами и / слэшами", "ru", "en", Backend::Local);
        assert!(!key.contains('/'));
        assert!(!key.contains(' '));
        assert!(key.starts_with("ru-en-"));
    }

    #[test]
    fn cache_dir_lives_next_to_the_program() {
        let dir = cache_dir(Path::new("C:/Capri"));
        assert_eq!(dir, Path::new("C:/Capri/cache/translate"));
    }

    #[test]
    fn no_provider_is_reported_honestly() {
        // Конфиг по умолчанию: локальный ИИ выключен, ключа нет, Libre нет.
        let mut cfg = Config::default();
        cfg.local_ai = false;
        cfg.anthropic_api_key = String::new();
        cfg.translate_libretranslate_url = String::new();
        let translator = Translator::new(&cfg, None);
        let error = translator.backend().unwrap_err();
        assert!(
            error.contains("нечем"),
            "сообщение должно объяснять причину: {error}"
        );
    }

    #[test]
    fn libre_wins_when_configured() {
        let mut cfg = Config::default();
        cfg.translate_libretranslate_url = "http://127.0.0.1:5000".into();
        let translator = Translator::new(&cfg, None);
        assert_eq!(translator.backend(), Ok(Backend::Libre));
    }

    #[test]
    fn local_is_used_when_enabled() {
        let mut cfg = Config::default();
        cfg.local_ai = true;
        cfg.translate_libretranslate_url = String::new();
        let translator = Translator::new(&cfg, None);
        assert_eq!(translator.backend(), Ok(Backend::Local));
    }

    #[test]
    fn same_language_needs_no_translation() {
        // Русский текст, цель — русский: отдаём как есть и не ходим в сеть.
        let mut cfg = Config::default();
        cfg.local_ai = false;
        cfg.anthropic_api_key = String::new();
        cfg.translate_libretranslate_url = String::new();
        let translator = Translator::new(&cfg, None);

        let result = translator
            .translate("Привет, мир", Some("ru"), "ru")
            .expect("перевод «в себя» не должен требовать провайдера");
        assert_eq!(result.text, "Привет, мир");
        assert_eq!(result.from, "ru");
        assert_eq!(result.to, "ru");
        assert!(!result.cached);
    }
}
