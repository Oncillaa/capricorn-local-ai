//! Горячая клавиша: разбор строки вида `Ctrl+Alt+C` и её регистрация.
//!
//! Почему не `global-hotkey`: тот поднимает собственный поток с собственной
//! обработкой сообщений, а у нас уже есть окно и цикл сообщений — Win32
//! `RegisterHotKey(hwnd, …)` кладёт `WM_HOTKEY` прямо в него. Эффект тот же,
//! зависимость и поток — лишние. По той же причине не тянем и `hotkey-parse`:
//! разбор строки — тридцать строк кода, зато свои тесты и свои сообщения
//! об ошибках по-русски.

/// Модификаторы (`MOD_*` из Win32).
pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_WIN: u32 = 0x0008;

/// Не повторять, пока клавишу держат: меню не должно мигать.
pub const MOD_NOREPEAT: u32 = 0x4000;

/// Разобранное сочетание клавиш.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    /// Что зажато: `Ctrl`, `Alt`, `Shift`, `Win` в виде битов `MOD_*`.
    pub modifiers: u32,
    /// Виртуальный код клавиши (`VK_*`).
    pub key: u32,
}

/// Разобрать строку из `config.json`. `Err` — что именно не так, по-русски.
pub fn parse(text: &str) -> Result<Hotkey, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("клавиша не задана".to_string());
    }

    let mut modifiers = 0u32;
    let mut key = None;
    for part in text.split('+') {
        let part = part.trim().to_ascii_lowercase();
        if part.is_empty() {
            return Err(format!("в «{text}» лишний плюс"));
        }
        match part.as_str() {
            "ctrl" | "control" => modifiers |= MOD_CONTROL,
            "alt" => modifiers |= MOD_ALT,
            "shift" => modifiers |= MOD_SHIFT,
            "win" | "super" | "meta" => modifiers |= MOD_WIN,
            other => {
                if key.is_some() {
                    return Err(format!("в «{text}» две клавиши, нужна одна"));
                }
                match named_key(other) {
                    Some(code) => key = Some(code),
                    None => return Err(format!("не знаю клавишу «{other}»")),
                }
            }
        }
    }

    let key = key.ok_or_else(|| format!("в «{text}» нет самой клавиши, только модификаторы"))?;
    if modifiers == 0 {
        // Без модификатора одна буква перехватила бы обычный набор текста.
        return Err("нужен хотя бы один модификатор: Ctrl, Alt, Shift или Win".to_string());
    }
    Ok(Hotkey { modifiers, key })
}

/// Виртуальный код по имени клавиши.
fn named_key(name: &str) -> Option<u32> {
    // Одна буква или цифра: её ASCII-код совпадает с `VK_`.
    let mut chars = name.chars();
    if let (Some(single), None) = (chars.next(), chars.next()) {
        let upper = single.to_ascii_uppercase();
        if upper.is_ascii_alphanumeric() {
            return Some(upper as u32);
        }
    }

    let code = match name {
        "space" => 0x20,
        "tab" => 0x09,
        "enter" | "return" => 0x0D,
        "esc" | "escape" => 0x1B,
        "backspace" => 0x08,
        "insert" => 0x2D,
        "delete" | "del" => 0x2E,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" | "pgup" => 0x21,
        "pagedown" | "pgdn" => 0x22,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        _ => {
            // F1…F24.
            let number: u32 = name.strip_prefix('f')?.parse().ok()?;
            if !(1..=24).contains(&number) {
                return None;
            }
            0x70 + number - 1
        }
    };
    Some(code)
}

/// Как сочетание выглядит для человека: `Ctrl+Alt+C`.
pub fn describe(hotkey: Hotkey) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if hotkey.modifiers & MOD_CONTROL != 0 {
        parts.push("Ctrl");
    }
    if hotkey.modifiers & MOD_ALT != 0 {
        parts.push("Alt");
    }
    if hotkey.modifiers & MOD_SHIFT != 0 {
        parts.push("Shift");
    }
    if hotkey.modifiers & MOD_WIN != 0 {
        parts.push("Win");
    }
    let key = if (0x30..=0x5A).contains(&hotkey.key) {
        // Буквы и цифры печатаются как есть.
        char::from_u32(hotkey.key).map(String::from).unwrap_or_default()
    } else {
        format!("VK{:02X}", hotkey.key)
    };
    parts.push(&key);
    parts.join("+")
}

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use windows::Win32::Foundation::{GetLastError, HWND, ERROR_HOTKEY_ALREADY_REGISTERED};
    use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};

    /// Идентификатор нашего сочетания. Любое число 0…0xBFFF.
    pub const HOTKEY_ID: i32 = 0x0CA9;

    /// Ошибка «клавишу уже заняли».
    const ALREADY_TAKEN: u32 = ERROR_HOTKEY_ALREADY_REGISTERED.0;

    /// Повесить сочетание на окно `hwnd`: `WM_HOTKEY` придёт в его обработчик.
    pub fn register(hwnd: HWND, hotkey: Hotkey) -> Result<(), String> {
        let ok = unsafe {
            RegisterHotKey(
                Some(hwnd),
                HOTKEY_ID,
                windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS(
                    hotkey.modifiers | MOD_NOREPEAT,
                ),
                hotkey.key,
            )
        };
        if ok.is_ok() {
            return Ok(());
        }
        let code = unsafe { GetLastError().0 };
        if code == ALREADY_TAKEN {
            return Err(format!(
                "клавиша {} занята другой программой — выбери другую в config.json",
                describe(hotkey)
            ));
        }
        Err(format!("Windows не отдала клавишу {} (код {code})", describe(hotkey)))
    }

    /// Снять сочетание. Ошибку глотаем: снимать нечего — не беда.
    pub fn unregister(hwnd: HWND) {
        let _ = unsafe { UnregisterHotKey(Some(hwnd), HOTKEY_ID) };
    }
}

#[cfg(windows)]
pub use windows_impl::{register, unregister, HOTKEY_ID};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_default_hotkey() {
        let hotkey = parse("Ctrl+Alt+C").unwrap();
        assert_eq!(hotkey.modifiers, MOD_CONTROL | MOD_ALT);
        assert_eq!(hotkey.key, u32::from(b'C'));
    }

    #[test]
    fn order_and_case_do_not_matter() {
        let first = parse("ctrl+alt+c").unwrap();
        let second = parse("ALT + CTRL + C").unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn modifier_spellings_are_interchangeable() {
        assert_eq!(parse("control+shift+a").unwrap().modifiers, MOD_CONTROL | MOD_SHIFT);
        assert_eq!(parse("super+alt+a").unwrap().modifiers, MOD_WIN | MOD_ALT);
        assert_eq!(parse("win+alt+a").unwrap().modifiers, MOD_WIN | MOD_ALT);
    }

    #[test]
    fn special_keys_are_known() {
        assert_eq!(parse("ctrl+alt+f12").unwrap().key, 0x7B);
        assert_eq!(parse("ctrl+space").unwrap().key, 0x20);
        assert_eq!(parse("ctrl+alt+delete").unwrap().key, 0x2E);
        assert_eq!(parse("ctrl+esc").unwrap().key, 0x1B);
    }

    #[test]
    fn a_bare_key_is_refused() {
        // Иначе Капри съедала бы обычный набор текста.
        let error = parse("c").unwrap_err();
        assert!(error.contains("модификатор"), "сообщение: {error}");
    }

    #[test]
    fn nonsense_is_refused_with_a_readable_reason() {
        assert!(parse("").unwrap_err().contains("не задана"));
        assert!(parse("ctrl+").unwrap_err().contains("плюс"));
        assert!(parse("ctrl+alt").unwrap_err().contains("нет самой клавиши"));
        assert!(parse("ctrl+alt+ж").unwrap_err().contains("не знаю клавишу"));
        assert!(parse("ctrl+a+b").unwrap_err().contains("две клавиши"));
        assert!(parse("ctrl+f25").unwrap_err().contains("не знаю клавишу"));
    }

    #[test]
    fn describes_itself_back() {
        assert_eq!(describe(parse("ctrl+alt+c").unwrap()), "Ctrl+Alt+C");
        assert_eq!(describe(parse("shift+win+space").unwrap()), "Shift+Win+VK20");
    }
}
