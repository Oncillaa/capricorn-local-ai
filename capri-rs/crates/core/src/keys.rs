//! Разбор клавиш: «вин таб», «контрол шифт эскейп», «альт эф четыре», «win+tab».
//!
//! Vosk пишет клавиши кириллицей, Google — латиницей, поэтому словарей два.

use crate::platform::KeyStroke;
use crate::util::{norm, parse_number};
use once_cell::sync::Lazy;
use std::collections::HashMap;

pub const VK_WIN: u16 = 0x5B;
pub const VK_CTRL: u16 = 0x11;
pub const VK_ALT: u16 = 0x12;
pub const VK_SHIFT: u16 = 0x10;
pub const VK_TAB: u16 = 0x09;
pub const VK_ENTER: u16 = 0x0D;
pub const VK_ESC: u16 = 0x1B;
pub const VK_SPACE: u16 = 0x20;
pub const VK_BACKSPACE: u16 = 0x08;
pub const VK_DELETE: u16 = 0x2E;
pub const VK_INSERT: u16 = 0x2D;
pub const VK_HOME: u16 = 0x24;
pub const VK_END: u16 = 0x23;
pub const VK_PAGEUP: u16 = 0x21;
pub const VK_PAGEDOWN: u16 = 0x22;
pub const VK_LEFT: u16 = 0x25;
pub const VK_UP: u16 = 0x26;
pub const VK_RIGHT: u16 = 0x27;
pub const VK_DOWN: u16 = 0x28;
pub const VK_PRINTSCREEN: u16 = 0x2C;
pub const VK_CAPSLOCK: u16 = 0x14;
pub const VK_MENU: u16 = 0x5D;
pub const VK_F1: u16 = 0x70;

/// Именованные клавиши: (имя, код, «расширенная» ли).
static NAMED: Lazy<HashMap<&'static str, KeyStroke>> = Lazy::new(|| {
    let mut m = HashMap::new();
    let mut add = |name: &'static str, vk: u16, extended: bool| {
        m.insert(name, KeyStroke { vk, extended });
    };
    add("win", VK_WIN, true);
    add("ctrl", VK_CTRL, false);
    add("alt", VK_ALT, false);
    add("shift", VK_SHIFT, false);
    add("tab", VK_TAB, false);
    add("enter", VK_ENTER, false);
    add("esc", VK_ESC, false);
    add("space", VK_SPACE, false);
    add("backspace", VK_BACKSPACE, false);
    add("delete", VK_DELETE, true);
    add("insert", VK_INSERT, true);
    add("home", VK_HOME, true);
    add("end", VK_END, true);
    add("pageup", VK_PAGEUP, true);
    add("pagedown", VK_PAGEDOWN, true);
    add("left", VK_LEFT, true);
    add("up", VK_UP, true);
    add("right", VK_RIGHT, true);
    add("down", VK_DOWN, true);
    add("printscreen", VK_PRINTSCREEN, true);
    add("capslock", VK_CAPSLOCK, false);
    add("menu", VK_MENU, true);
    m
});

/// Как эти клавиши произносят.
const KEY_WORDS: &[(&str, &str)] = &[
    ("win", "вин винд винда виндовс виндоус виндоуз виндос виндус винды винда windows window win wind"),
    ("ctrl", "контрол контроль контрл кнтрл ктрл контр ctrl control ctl"),
    ("alt", "альт алт alt"),
    ("shift", "шифт shift"),
    ("tab", "таб табуляция tab"),
    ("enter", "энтер ентер enter ввод return"),
    ("esc", "эскейп искейп эскейф эск esc escape"),
    ("space", "пробел space спейс"),
    ("backspace", "бэкспейс бекспейс backspace"),
    ("delete", "делит дилит делейт delete del дел"),
    ("insert", "инсерт insert"),
    ("home", "хоум home"),
    ("end", "энд end"),
    ("printscreen", "принтскрин printscreen prtsc"),
    ("capslock", "капслок capslock"),
    ("left", "влево лево налево left"),
    ("right", "вправо право направо right"),
    ("up", "вверх наверх up"),
    ("down", "вниз down"),
    ("menu", "меню"),
];

static WORD_TO_KEY: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    for (key, words) in KEY_WORDS {
        for w in words.split_whitespace() {
            m.insert(w, *key);
        }
    }
    m
});

const LETTER_NAMES: &[(&str, char)] = &[
    ("эй", 'a'), ("би", 'b'), ("си", 'c'), ("ди", 'd'), ("и", 'e'), ("эф", 'f'), ("джи", 'g'),
    ("эйч", 'h'), ("ай", 'i'), ("джей", 'j'), ("кей", 'k'), ("эл", 'l'), ("эм", 'm'), ("эн", 'n'),
    ("оу", 'o'), ("пи", 'p'), ("кью", 'q'), ("ар", 'r'), ("эс", 's'), ("ти", 't'), ("ю", 'u'),
    ("ви", 'v'), ("дабл ю", 'w'), ("дабл-ю", 'w'), ("экс", 'x'), ("икс", 'x'), ("уай", 'y'),
    ("игрек", 'y'), ("зед", 'z'), ("зет", 'z'),
    ("а", 'a'), ("бэ", 'b'), ("б", 'b'), ("цэ", 'c'), ("ц", 'c'), ("дэ", 'd'), ("д", 'd'),
    ("е", 'e'), ("гэ", 'g'), ("г", 'g'), ("ха", 'h'), ("ка", 'k'), ("к", 'k'), ("эль", 'l'),
    ("л", 'l'), ("м", 'm'), ("н", 'n'), ("о", 'o'), ("пэ", 'p'), ("п", 'p'), ("эр", 'r'),
    ("р", 'r'), ("с", 's'), ("тэ", 't'), ("т", 't'), ("у", 'u'), ("вэ", 'v'), ("в", 'v'),
    ("зэ", 'z'), ("з", 'z'),
];

const SKIP_WORDS: &[&str] = &[
    "и", "плюс", "кнопку", "кнопка", "кнопки", "клавишу", "клавиши", "клавиша", "клавиш",
    "сочетание", "сочетания", "комбинацию", "комбинация", "стрелку", "стрелка", "стрелочку",
    "на", "клавиатуре", "одновременно", "мне", "пожалуйста", "plus", "and",
];

fn letter_by_name(w: &str) -> Option<char> {
    LETTER_NAMES.iter().find(|(n, _)| *n == w).map(|(_, c)| *c)
}

fn vk_for_char(c: char) -> u16 {
    c.to_ascii_uppercase() as u16
}

fn named(name: &str) -> Option<KeyStroke> {
    NAMED.get(name).copied()
}

fn fkey(n: u32) -> Option<KeyStroke> {
    if (1..=24).contains(&n) {
        Some(KeyStroke { vk: VK_F1 + (n as u16 - 1), extended: false })
    } else {
        None
    }
}

/// «wintab», «альттаб», «ctrlshiftesc» → список клавиш.
///
/// Одиночная буква/цифра допустима только в конце (иначе «и» съест слово).
fn split_glued(word: &str) -> Option<Vec<KeyStroke>> {
    let mut names: Vec<&str> = WORD_TO_KEY
        .keys()
        .copied()
        .filter(|w| w.chars().count() > 1)
        .collect();
    names.sort_by_key(|w| std::cmp::Reverse(w.chars().count()));
    go_split(word, &names)
}

fn go_split(rest: &str, names: &[&str]) -> Option<Vec<KeyStroke>> {
    if rest.is_empty() {
        return Some(Vec::new());
    }
    for n in names {
        if let Some(tail_str) = rest.strip_prefix(n) {
            if let Some(mut tail) = go_split(tail_str, names) {
                let key = named(WORD_TO_KEY.get(n).copied().unwrap_or(n))?;
                let mut out = vec![key];
                out.append(&mut tail);
                return Some(out);
            }
        }
    }
    // «ф5», «f12»
    let chars: Vec<char> = rest.chars().collect();
    if chars.len() >= 2 && matches!(chars[0], 'f' | 'ф') && chars[1].is_ascii_digit() {
        let digits: String = chars[1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(n) = digits.parse::<u32>() {
            if let Some(mut tail) = go_split(&chars[1 + digits.chars().count()..].iter().collect::<String>(), names) {
                let mut out = vec![fkey(n)?];
                out.append(&mut tail);
                return Some(out);
            }
        }
    }
    if chars.len() == 1 && (chars[0].is_ascii_alphanumeric()) {
        return Some(vec![KeyStroke { vk: vk_for_char(chars[0]), extended: false }]);
    }
    None
}

/// Разобрать фразу в сочетание клавиш: `Ok(keys)` либо `Err(слово)`,
/// которое не удалось понять.
pub fn parse_keys(text: &str) -> Result<Vec<KeyStroke>, String> {
    let mut t = norm(text);
    for (a, b) in [
        ("пейдж ап", "pageup"),
        ("page up", "pageup"),
        ("пейдж даун", "pagedown"),
        ("page down", "pagedown"),
        ("принт скрин", "printscreen"),
        ("print screen", "printscreen"),
        ("капс лок", "capslock"),
        ("caps lock", "capslock"),
        ("дабл ю", "дабл-ю"),
        ("бэк спейс", "backspace"),
        ("back space", "backspace"),
    ] {
        t = t.replace(a, b);
    }
    t = t.replace(['+', '-', '_', ',', '.'], " ");

    let words: Vec<&str> = t.split_whitespace().collect();
    let mut keys: Vec<KeyStroke> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        let next = words.get(i + 1).copied().unwrap_or("");
        if let Some(name) = WORD_TO_KEY.get(w) {
            if let Some(k) = named(name) {
                keys.push(k);
            }
        } else if let Some(k) = named(w) {
            keys.push(k);
        } else if w.len() >= 2 && matches!(w.chars().next().unwrap(), 'f' | 'ф')
            && w[1..].chars().all(|c| c.is_ascii_digit())
        {
            match w[1..].parse::<u32>().ok().and_then(fkey) {
                Some(k) => keys.push(k),
                None => return Err(w.to_string()),
            }
        } else if matches!(w, "эф" | "ф" | "f") {
            let num = next.parse::<i64>().ok().or_else(|| parse_number(next));
            match num.and_then(|n| fkey(n as u32)) {
                Some(k) => {
                    keys.push(k);
                    i += 1;
                }
                None => return Err(w.to_string()),
            }
        } else if let Some(c) = letter_by_name(w) {
            // «и» — это ещё и союз; считаем его буквой только в конце фразы.
            if w == "и" && !keys.is_empty() && i + 1 < words.len() {
                i += 1;
                continue;
            }
            keys.push(KeyStroke { vk: vk_for_char(c), extended: false });
        } else if w.chars().count() == 1 && w.chars().next().unwrap().is_ascii_alphanumeric() {
            keys.push(KeyStroke { vk: vk_for_char(w.chars().next().unwrap()), extended: false });
        } else if let Some(n) = parse_number(w) {
            if (0..10).contains(&n) {
                let c = char::from_digit(n as u32, 10).unwrap_or('0');
                keys.push(KeyStroke { vk: vk_for_char(c), extended: false });
            } else {
                return Err(w.to_string());
            }
        } else if !SKIP_WORDS.contains(&w) {
            match split_glued(w) {
                Some(mut glued) => keys.append(&mut glued),
                None => return Err(w.to_string()),
            }
        }
        i += 1;
    }
    if keys.is_empty() {
        Err(text.to_string())
    } else {
        Ok(keys)
    }
}

/// Человеческое название сочетания для озвучки: «вин + таб».
pub fn describe(keys: &[KeyStroke]) -> String {
    let mut names: Vec<String> = Vec::new();
    for k in keys {
        if let Some((name, _)) = NAMED
            .iter()
            .find(|(_, v)| v.vk == k.vk && v.extended == k.extended)
            .or_else(|| NAMED.iter().find(|(_, v)| v.vk == k.vk))
        {
            names.push(match *name {
                "win" => "вин".to_string(),
                "ctrl" => "контрол".to_string(),
                "alt" => "альт".to_string(),
                "shift" => "шифт".to_string(),
                other => other.to_string(),
            });
        } else if (VK_F1..VK_F1 + 24).contains(&k.vk) {
            names.push(format!("эф {}", k.vk - VK_F1 + 1));
        } else if (0x30..=0x5A).contains(&k.vk) {
            names.push((k.vk as u8 as char).to_string());
        }
    }
    names.join(" + ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_russian_keys() {
        let k = parse_keys("вин таб").unwrap();
        assert_eq!(k.len(), 2);
        assert_eq!(k[0].vk, VK_WIN);
        assert_eq!(k[1].vk, VK_TAB);
    }

    #[test]
    fn parses_combos() {
        let k = parse_keys("контрол шифт эскейп").unwrap();
        assert_eq!(k.iter().map(|k| k.vk).collect::<Vec<_>>(), vec![VK_CTRL, VK_SHIFT, VK_ESC]);
        let k = parse_keys("альт эф четыре").unwrap();
        assert_eq!(k[1].vk, VK_F1 + 3);
    }

    #[test]
    fn parses_glued() {
        let k = parse_keys("wintab").unwrap();
        assert_eq!(k.len(), 2);
        assert!(!parse_keys("абракадабра").is_ok());
    }
}
