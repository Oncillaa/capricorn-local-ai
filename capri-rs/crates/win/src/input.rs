//! Ввод: клавиши, печать текста, прокрутка, вкладки.

use capri_core::keys::{
    VK_BACKSPACE, VK_CTRL, VK_DOWN, VK_END, VK_HOME, VK_PAGEDOWN, VK_PAGEUP, VK_SHIFT, VK_SPACE,
    VK_TAB,
};
use capri_core::platform::{KeyStroke, ScrollDirection, TabCommand};

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSEINPUT, MOUSEEVENTF_WHEEL,
        VIRTUAL_KEY,
    };

    fn key_input(vk: u16, extended: bool, up: bool) -> INPUT {
        let mut flags = KEYBD_EVENT_FLAGS(0);
        if extended {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if up {
            flags |= KEYEVENTF_KEYUP;
        }
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn unicode_input(unit: u16, up: bool) -> INPUT {
        let mut flags = KEYEVENTF_UNICODE;
        if up {
            flags |= KEYEVENTF_KEYUP;
        }
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: unit,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    pub fn send_keys(keys: &[KeyStroke]) -> Result<(), String> {
        if keys.is_empty() {
            return Err("нечего нажимать".into());
        }
        let mut inputs: Vec<INPUT> = Vec::with_capacity(keys.len() * 2);
        // Зажимаем по порядку...
        for k in keys {
            inputs.push(key_input(k.vk, k.extended, false));
        }
        // ...и отпускаем в обратном порядке, чтобы работали сочетания.
        for k in keys.iter().rev() {
            inputs.push(key_input(k.vk, k.extended, true));
        }
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent as usize != inputs.len() {
            return Err("Windows не приняла нажатие клавиш".into());
        }
        Ok(())
    }

    pub fn type_text(text: &str) -> Result<(), String> {
        // KEYEVENTF_UNICODE: работает и с русскими буквами, и с эмодзи.
        let units: Vec<u16> = text.encode_utf16().collect();
        for chunk in units.chunks(40) {
            let mut inputs: Vec<INPUT> = Vec::with_capacity(chunk.len() * 2);
            for &u in chunk {
                inputs.push(unicode_input(u, false));
                inputs.push(unicode_input(u, true));
            }
            let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
            if sent as usize != inputs.len() {
                return Err("не смогла напечатать текст".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(6));
        }
        Ok(())
    }

    pub fn scroll(steps: i32) -> Result<(), String> {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: (steps * 120) as u32,
                    dwFlags: MOUSEEVENTF_WHEEL,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
        if sent == 0 {
            return Err("прокрутка не сработала".into());
        }
        Ok(())
    }
}

/// Нажать сочетание клавиш (зажать по порядку, отпустить в обратном).
pub fn send_keys(keys: &[KeyStroke]) -> Result<(), String> {
    #[cfg(windows)]
    {
        return imp::send_keys(keys);
    }
    #[cfg(not(windows))]
    Err("только для Windows".into())
}

/// Напечатать текст в активное окно (поддерживает кириллицу).
pub fn type_text(text: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        return imp::type_text(text);
    }
    #[cfg(not(windows))]
    {
        let _ = text;
        Err("только для Windows".into())
    }
}

/// Прокрутить страницу: вниз/вверх на шаг или до конца/начала.
pub fn scroll(dir: ScrollDirection, steps: u32) -> Result<(), String> {
    let r = match dir {
        ScrollDirection::Down => scroll_wheel(-(steps.min(30) as i32)),
        ScrollDirection::Up => scroll_wheel(steps.min(30) as i32),
        ScrollDirection::Bottom => send_keys(&[KeyStroke::new(VK_CTRL), KeyStroke::new(VK_END)]),
        ScrollDirection::Top => send_keys(&[KeyStroke::new(VK_CTRL), KeyStroke::new(VK_HOME)]),
    };
    r
}

fn scroll_wheel(steps: i32) -> Result<(), String> {
    #[cfg(windows)]
    {
        return imp::scroll(steps);
    }
    #[cfg(not(windows))]
    {
        let _ = steps;
        Err("только для Windows".into())
    }
}

/// Управление вкладками браузера.
pub fn tab_command(cmd: TabCommand) -> Result<(), String> {
    let keys: &[KeyStroke] = match cmd {
        TabCommand::New => &[KeyStroke::new(VK_CTRL), KeyStroke::new('T' as u16)],
        TabCommand::Close => &[KeyStroke::new(VK_CTRL), KeyStroke::new('W' as u16)],
        TabCommand::ReopenClosed => &[
            KeyStroke::new(VK_CTRL),
            KeyStroke { vk: VK_SHIFT, extended: false },
            KeyStroke::new('T' as u16),
        ],
        TabCommand::Next => &[KeyStroke::new(VK_CTRL), KeyStroke::new(VK_TAB)],
        TabCommand::Prev => &[
            KeyStroke::new(VK_CTRL),
            KeyStroke { vk: VK_SHIFT, extended: false },
            KeyStroke::new(VK_TAB),
        ],
    };
    send_keys(keys)
}

/// Клавиши, которые нужны другим модулям.
pub fn vk_page_down() -> u16 {
    VK_PAGEDOWN
}
pub fn vk_page_up() -> u16 {
    VK_PAGEUP
}
pub fn vk_space() -> u16 {
    VK_SPACE
}
pub fn vk_down() -> u16 {
    VK_DOWN
}
pub fn vk_backspace() -> u16 {
    VK_BACKSPACE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vk_codes_are_sane() {
        assert_eq!(vk_page_down(), 0x22);
        assert_eq!(vk_backspace(), 0x08);
    }
}
