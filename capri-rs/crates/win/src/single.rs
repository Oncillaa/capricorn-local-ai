//! Один экземпляр Капри на пользователя.
//!
//! Два запуска — это два окна, две иконки в трее и драка за одну и ту же
//! горячую клавишу. Поэтому второй экземпляр честно говорит «я уже тут» и
//! выходит, а не тихо ломает первый.

/// Права на единственный запуск. Пока объект жив, имя занято.
///
/// Проверку делает само ядро: `CreateMutexW` + `ERROR_ALREADY_EXISTS`, без
/// гонки «сначала проверь, потом займи».
pub struct InstanceGuard {
    #[cfg(windows)]
    handle: windows::Win32::Foundation::HANDLE,
}

impl InstanceGuard {
    /// Занять имя приложения.
    ///
    /// `None` — Капри уже запущена: имя занято другим процессом.
    pub fn acquire(name: &str) -> Option<Self> {
        #[cfg(windows)]
        {
            use windows::core::PCWSTR;
            use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
            use windows::Win32::System::Threading::CreateMutexW;

            let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            unsafe {
                // Права на мьютекс не нужны — важно только владение именем.
                let handle = CreateMutexW(None, true, PCWSTR(wide.as_ptr())).ok()?;
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    let _ = CloseHandle(handle);
                    return None;
                }
                Some(Self { handle })
            }
        }
        #[cfg(not(windows))]
        {
            let _ = name;
            Some(Self {})
        }
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

/// Попросить уже запущенную Капри показаться.
///
/// Второй запуск — это чаще всего «я забыл, где она»: вместо окна с
/// предупреждением просто зовём первый экземпляр. `false` — окна не нашли.
///
/// Класс окна и номер сообщения приходят снаружи: этой крепкой нечего знать
/// про интерфейс.
pub fn summon_running_instance(class: &str, message: u32) -> bool {
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};

        let class = HSTRING::from(class);
        unsafe {
            let Ok(hwnd) = FindWindowW(&class, None) else {
                return false;
            };
            if hwnd.is_invalid() {
                return false;
            }
            PostMessageW(Some(hwnd), message, Default::default(), Default::default()).is_ok()
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (class, message);
        false
    }
}

/// Сказать человеку, что Капри уже работает.
///
/// Молчаливый выход выглядит как поломка: человек жмёт второй раз и не
/// понимает, почему ничего не появилось.
pub fn already_running_notice(app_name: &str, text: &str) {
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND,
        };

        let caption = HSTRING::from(app_name);
        let body = HSTRING::from(text);
        unsafe {
            MessageBoxW(
                None,
                &body,
                &caption,
                MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND,
            );
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (app_name, text);
    }
}
