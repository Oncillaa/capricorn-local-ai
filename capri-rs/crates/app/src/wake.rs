//! Слово-будильник: «Капри», «каприз», «кепри» и прочие варианты.
//!
//! Варианты берём из настроек и дополняем «похожими по звучанию», чтобы
//! распознавание не сбивало обращение. Всё считается обычной строкой —
//! без регулярок в горячем пути.

/// Слова, которыми просят замолчать.
const STOP_WORDS: &[&str] = &[
    "стоп",
    "хватит",
    "молчи",
    "замолчи",
    "помолчи",
    "достаточно",
    "подожди",
    "остановись",
    "перестань",
    "заткнись",
    "умолкни",
];

#[derive(Clone)]
pub struct Wake {
    /// Все варианты обращения в нижнем регистре, длинные первыми.
    words: Vec<String>,
}

impl Wake {
    pub fn new(config_words: &[String]) -> Self {
        let mut words: Vec<String> = config_words
            .iter()
            .map(|w| w.trim().to_lowercase())
            .filter(|w| !w.is_empty())
            .collect();
        if words.is_empty() {
            words = vec!["капри".into(), "каприз".into(), "кепри".into()];
        }
        // Дополняем частыми ошибками распознавания.
        for extra in ["капри", "каприз", "капре", "кепри", "капли", "каприк", "каприи"] {
            if !words.iter().any(|w| w == extra) {
                words.push(extra.to_string());
            }
        }
        words.sort_by_key(|w| std::cmp::Reverse(w.chars().count()));
        words.dedup();
        Self { words }
    }

    /// Есть ли в тексте обращение.
    pub fn has(&self, text: &str) -> bool {
        let low = normalize(text);
        self.word_at(&low).is_some()
    }

    /// Убрать обращение из текста (и лишние знаки по краям).
    pub fn strip(&self, text: &str) -> String {
        let low = normalize(text);
        let mut rest = low.clone();
        // Отрезаем всё до последнего обращения: «эээ капри открой ютуб».
        if let Some((_start, end)) = self.last_word_at(&low) {
            let tail = &low[end..];
            if !tail.trim_matches(|c: char| c.is_whitespace() || ",.!?".contains(c)).is_empty() {
                rest = tail.to_string();
            }
        }
        for word in &self.words {
            rest = replace_word(&rest, word, " ");
        }
        rest.trim_matches(|c: char| c.is_whitespace() || ",.!?".contains(c))
            .to_string()
    }

    /// Похоже на просьбу замолчать?
    pub fn is_stop(&self, text: &str) -> bool {
        let low = normalize(text);
        low.split_whitespace().any(|word| {
            let word = word.trim_matches(|c: char| !c.is_alphanumeric());
            STOP_WORDS.contains(&word)
        })
    }

    fn word_at(&self, low: &str) -> Option<usize> {
        self.words.iter().position(|word| contains_word(low, word))
    }

    fn last_word_at(&self, low: &str) -> Option<(usize, usize)> {
        let mut found: Option<(usize, usize)> = None;
        for word in &self.words {
            let mut from = 0;
            while let Some(at) = low[from..].find(word.as_str()) {
                let start = from + at;
                let end = start + word.len();
                let before_ok = start == 0 || !low[..start].chars().last().is_some_and(char::is_alphanumeric);
                let after_ok = end >= low.len() || !low[end..].chars().next().is_some_and(char::is_alphanumeric);
                if before_ok && after_ok && found.is_none_or(|(s, _)| start > s) {
                    found = Some((start, end));
                }
                from = end;
                if from >= low.len() {
                    break;
                }
            }
        }
        found
    }
}

fn normalize(text: &str) -> String {
    text.to_lowercase().replace('ё', "е")
}

fn contains_word(haystack: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(at) = haystack[from..].find(needle) {
        let start = from + at;
        let end = start + needle.len();
        let before_ok = start == 0 || !haystack[..start].chars().last().is_some_and(char::is_alphanumeric);
        let after_ok = end >= haystack.len() || !haystack[end..].chars().next().is_some_and(char::is_alphanumeric);
        if before_ok && after_ok {
            return true;
        }
        from = end;
        if from >= haystack.len() {
            break;
        }
    }
    false
}

/// Заменить слово целиком (не трогая «капризы» внутри других слов).
fn replace_word(haystack: &str, needle: &str, with: &str) -> String {
    let mut out = String::with_capacity(haystack.len());
    let mut rest = haystack;
    loop {
        let Some(at) = rest.find(needle) else {
            out.push_str(rest);
            return out;
        };
        let end = at + needle.len();
        let before_ok = at == 0 || !rest[..at].chars().last().is_some_and(char::is_alphanumeric);
        let after_ok = end >= rest.len() || !rest[end..].chars().next().is_some_and(char::is_alphanumeric);
        out.push_str(&rest[..at]);
        if before_ok && after_ok {
            out.push_str(with);
        } else {
            out.push_str(needle);
        }
        rest = &rest[end..];
        if rest.is_empty() {
            return out;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wake() -> Wake {
        Wake::new(&["капри".into(), "каприз".into()])
    }

    #[test]
    fn spots_wake_word() {
        assert!(wake().has("Капри, открой ютуб"));
        assert!(wake().has("каприз"));
        assert!(!wake().has("капризы в моде"));
        assert!(!wake().has("открой ютуб"));
    }

    #[test]
    fn strips_wake_word() {
        assert_eq!(wake().strip("Капри, открой ютуб"), "открой ютуб");
        assert_eq!(wake().strip("эээ капри открой ютуб"), "открой ютуб");
        assert_eq!(wake().strip("капри"), "");
    }

    #[test]
    fn notices_stop_words() {
        assert!(wake().is_stop("стоп"));
        assert!(wake().is_stop("капри, замолчи!"));
        assert!(!wake().is_stop("открой ютуб"));
    }

    #[test]
    fn works_with_empty_config() {
        let empty = Wake::new(&[]);
        assert!(empty.has("кепри"));
    }
}

