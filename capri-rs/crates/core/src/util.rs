//! Мелкие текстовые утилиты: нормализация русской речи, числа прописью,
//! нечёткое сравнение (как `difflib` в Python-версии).

use once_cell::sync::Lazy;
use std::collections::HashMap;

/// «Капри, ОТКРОЙ ютюб!» → «капри открой ютюб»
pub fn norm(s: &str) -> String {
    let lowered = s.to_lowercase().replace('ё', "е");
    let mut out = String::with_capacity(lowered.len());
    for ch in lowered.chars() {
        let keep = ch.is_alphanumeric() || ch.is_whitespace() || matches!(ch, '.' | '-' | ':' | '/');
        out.push(if keep { ch } else { ' ' });
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

static TRANSLIT: Lazy<HashMap<char, &'static str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    for (ru, lat) in [
        ("а", "a"), ("б", "b"), ("в", "v"), ("г", "g"), ("д", "d"), ("е", "e"),
        ("з", "z"), ("и", "i"), ("й", "y"), ("к", "k"), ("л", "l"), ("м", "m"),
        ("н", "n"), ("о", "o"), ("п", "p"), ("р", "r"), ("с", "s"), ("т", "t"),
        ("у", "u"), ("ф", "f"), ("х", "h"), ("ц", "c"), ("ы", "y"),
        ("ё", "e"), ("ж", "zh"), ("ч", "ch"), ("ш", "sh"), ("щ", "sch"),
        ("ъ", ""), ("ь", ""), ("э", "e"), ("ю", "yu"), ("я", "ya"),
        (" ", " "), ("-", "-"),
    ] {
        m.insert(ru.chars().next().unwrap(), lat);
    }
    m
});

/// Русское слово латиницей — помогает сравнивать «ютуб» и «youtube».
pub fn translit(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.to_lowercase().chars() {
        match TRANSLIT.get(&c) {
            Some(lat) => out.push_str(lat),
            None => out.push(c),
        }
    }
    out
}

/// Звуковой скелет слова без гласных: «ютуб» и «youtube» → «itb».
pub fn sound_key(s: &str) -> String {
    let t: String = translit(s).chars().filter(|c| !c.is_whitespace()).collect();
    t.chars()
        .filter(|c| !matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y'))
        .collect()
}

/// Похоже ли одно слово на другое «на слух».
pub fn sounds_like(a: &str, b: &str) -> bool {
    let (ka, kb) = (sound_key(a), sound_key(b));
    !ka.is_empty() && !kb.is_empty() && ka == kb
}

/// Степень похожести строк 0.0…1.0.
pub fn similarity(a: &str, b: &str) -> f64 {
    strsim::normalized_levenshtein(&a.to_lowercase(), &b.to_lowercase())
}

/// Самый похожий вариант из списка (порог 0…1), как `difflib.get_close_matches`.
pub fn closest<'a, I>(query: &str, candidates: I, cutoff: f64) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut best: Option<(f64, String)> = None;
    for c in candidates {
        let score = similarity(query, c);
        if score >= cutoff && best.as_ref().map_or(true, |(s, _)| score > *s) {
            best = Some((score, c.to_string()));
        }
    }
    best.map(|(_, s)| s)
}

static NUMS: Lazy<HashMap<&'static str, i64>> = Lazy::new(|| {
    let mut m = HashMap::new();
    for (w, n) in [
        ("ноль", 0), ("один", 1), ("одна", 1), ("два", 2), ("две", 2), ("три", 3),
        ("четыре", 4), ("пять", 5), ("шесть", 6), ("семь", 7), ("восемь", 8), ("девять", 9),
        ("десять", 10), ("одиннадцать", 11), ("двенадцать", 12), ("тринадцать", 13),
        ("четырнадцать", 14), ("пятнадцать", 15), ("шестнадцать", 16), ("семнадцать", 17),
        ("восемнадцать", 18), ("девятнадцать", 19), ("двадцать", 20), ("тридцать", 30),
        ("сорок", 40), ("пятьдесят", 50), ("шестьдесят", 60), ("семьдесят", 70),
        ("восемьдесят", 80), ("девяносто", 90), ("сто", 100),
    ] {
        m.insert(w, n);
    }
    m
});

/// «пятьдесят» → 50, «на максимум» → 100.
pub fn parse_number(text: &str) -> Option<i64> {
    let t = text.to_lowercase();
    if let Some(digits) = t
        .split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())
    {
        if let Ok(n) = digits.parse::<i64>() {
            return Some(n);
        }
    }
    let mut total: Option<i64> = None;
    for w in t.split_whitespace() {
        if let Some(n) = NUMS.get(w) {
            total = Some(total.unwrap_or(0) + n);
        } else if total.is_some() {
            break;
        }
    }
    if total.is_some() {
        return total;
    }
    if t.contains("максимум") || t.contains("на полную") || t.contains("на всю") {
        return Some(100);
    }
    if t.contains("половин") {
        return Some(50);
    }
    if t.contains("минимум") {
        return Some(0);
    }
    None
}

/// «слово» → «слово» без хвоста прилагательного/падежа — грубый стеммер
/// (нужен для поиска папок и программ по разным формам).
pub fn stem(name: &str) -> String {
    let mut s = name.to_lowercase().replace('ё', "е");
    for suffix in [
        "ами", "ями", "ого", "его", "ому", "ему", "ов", "ев", "ей", "ой", "ом", "ем", "ым",
        "им", "ою", "ею", "ах", "ях", "ам", "ям", "ую", "юю", "ая", "яя", "ые", "ие", "ый",
        "ий", "ое", "ее", "а", "я", "ы", "и", "е", "у", "ю", "о",
    ] {
        if s.len() > suffix.len() + 3 && s.ends_with(suffix) {
            s.truncate(s.len() - suffix.len());
            break;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_speech() {
        assert_eq!(norm("Капри, открой Ютуб!!"), "капри открой ютуб");
        assert_eq!(norm("поставь на 3:14"), "поставь на 3:14");
    }

    #[test]
    fn understands_numbers() {
        assert_eq!(parse_number("поставь на 3:14"), Some(3));
        assert_eq!(parse_number("сорок пять"), Some(45));
        assert_eq!(parse_number("на максимум"), Some(100));
        assert_eq!(parse_number("половину"), Some(50));
    }

    #[test]
    fn compares_sounds() {
        assert!(sounds_like("ютуб", "youtube"));
        // Одна лишняя буква на девять — это 0,89, порог похожести в движке 0,85.
        assert!(similarity("телеграм", "телеграмм") > 0.85);
    }
}
