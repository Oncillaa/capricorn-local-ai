//! Окна: перечисление, поиск, переключение, закрытие, сворачивание.

use capri_core::platform::WindowInfo;
use capri_core::util::norm;

#[cfg(windows)]
mod imp {
    use super::*;
    use std::sync::Mutex;

    use windows::core::{BOOL, PWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, MAX_PATH, RECT};
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::System::Threading::{
        AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowLongW, GetWindowRect,
        GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        PostMessageW, SetForegroundWindow, ShowWindow, GWL_EXSTYLE, SW_MINIMIZE, SW_RESTORE,
        WM_CLOSE, WS_EX_TOOLWINDOW,
    };

    static COLLECT: Mutex<Vec<WindowInfo>> = Mutex::new(Vec::new());

    unsafe extern "system" fn collect_cb(hwnd: HWND, _: LPARAM) -> BOOL {
        if let Some(info) = describe(hwnd) {
            if let Ok(mut list) = COLLECT.lock() {
                list.push(info);
            }
        }
        BOOL(1)
    }

    pub fn enum_windows() -> Vec<WindowInfo> {
        if let Ok(mut list) = COLLECT.lock() {
            list.clear();
        }
        unsafe {
            let _ = EnumWindows(Some(collect_cb), LPARAM(0));
        }
        COLLECT
            .lock()
            .map(|mut l| std::mem::take(&mut *l))
            .unwrap_or_default()
    }

    pub fn describe(hwnd: HWND) -> Option<WindowInfo> {
        unsafe {
            if !IsWindowVisible(hwnd).as_bool() {
                return None;
            }
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                &mut cloaked as *mut u32 as *mut core::ffi::c_void,
                std::mem::size_of::<u32>() as u32,
            );
            if cloaked != 0 {
                return None;
            }
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
                return None;
            }
            let len = GetWindowTextLengthW(hwnd);
            if len <= 0 {
                return None;
            }
            let mut buf = [0u16; 512];
            let copied = GetWindowTextW(hwnd, &mut buf).max(0) as usize;
            let title = crate::util::from_wide(&buf[..copied]);

            let mut class_buf = [0u16; 256];
            let class_len = GetClassNameW(hwnd, &mut class_buf).max(0) as usize;
            let class = crate::util::from_wide(&class_buf[..class_len]);

            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));

            Some(WindowInfo {
                handle: hwnd.0 as isize,
                title,
                class,
                exe: process_exe(pid),
                pid,
            })
        }
    }

    pub fn process_exe(pid: u32) -> Option<String> {
        unsafe {
            let handle =
                OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, false, pid).ok()?;
            let mut buf = [0u16; MAX_PATH as usize];
            let mut size = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut size,
            );
            let _ = windows::Win32::Foundation::CloseHandle(handle);
            if ok.is_ok() {
                Some(crate::util::from_wide(&buf[..size as usize]))
            } else {
                None
            }
        }
    }

    pub fn foreground() -> Option<WindowInfo> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.0.is_null() {
                None
            } else {
                describe(hwnd)
            }
        }
    }

    pub fn focus(handle: isize) -> bool {
        unsafe {
            let hwnd = HWND(handle as *mut _);
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            if SetForegroundWindow(hwnd).as_bool() {
                return true;
            }
            let fg = GetForegroundWindow();
            let fg_thread = GetWindowThreadProcessId(fg, None);
            let me = GetCurrentThreadId();
            let attached = AttachThreadInput(me, fg_thread, true).as_bool();
            let ok = SetForegroundWindow(hwnd).as_bool();
            if attached {
                let _ = AttachThreadInput(me, fg_thread, false);
            }
            ok
        }
    }

    pub fn close(handle: isize) {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(handle as *mut _)),
                WM_CLOSE,
                windows::Win32::Foundation::WPARAM(0),
                windows::Win32::Foundation::LPARAM(0),
            );
        }
    }

    pub fn minimize(handle: isize) {
        unsafe {
            let _ = ShowWindow(HWND(handle as *mut _), SW_MINIMIZE);
        }
    }

    pub fn minimize_all() -> bool {
        unsafe {
            let class = windows::core::w!("Shell_TrayWnd");
            if let Ok(tray) = windows::Win32::UI::WindowsAndMessaging::FindWindowW(
                class,
                windows::core::PCWSTR::null(),
            ) {
                if !tray.0.is_null() {
                    windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                        tray,
                        0x0111, // WM_COMMAND
                        Some(windows::Win32::Foundation::WPARAM(419)),
                        Some(windows::Win32::Foundation::LPARAM(0)),
                    );
                    return true;
                }
            }
            false
        }
    }

    pub fn window_rect(handle: isize) -> Option<RECT> {
        unsafe {
            let mut rect = RECT::default();
            if GetWindowRect(HWND(handle as *mut _), &mut rect).is_ok() {
                Some(rect)
            } else {
                None
            }
        }
    }
}

/// Все видимые окна с заголовками.
pub fn list() -> Vec<WindowInfo> {
    #[cfg(windows)]
    {
        return imp::enum_windows();
    }
    #[cfg(not(windows))]
    Vec::new()
}

/// Активное окно.
pub fn foreground() -> Option<WindowInfo> {
    #[cfg(windows)]
    {
        return imp::foreground();
    }
    #[cfg(not(windows))]
    None
}

/// Найти окно по заголовку: точное вхождение, затем похожее.
pub fn find_by_title(query: &str) -> Option<WindowInfo> {
    let q = norm(query);
    if q.is_empty() {
        return None;
    }
    let windows = list();
    if let Some(w) = windows.iter().find(|w| norm(&w.title) == q) {
        return Some(w.clone());
    }
    windows
        .iter()
        .find(|w| norm(&w.title).contains(&q))
        .or_else(|| {
            windows
                .iter()
                .find(|w| capri_core::util::similarity(&norm(&w.title), &q) > 0.6)
        })
        .cloned()
}

/// Найти окно процесса: «telegram», «chrome.exe», «spotify».
pub fn find_by_process(process: &str) -> Option<WindowInfo> {
    let needle = process.to_lowercase().replace(".exe", "");
    list().into_iter().find(|w| {
        w.exe
            .as_ref()
            .map(|e| e.to_lowercase().contains(&needle))
            .unwrap_or(false)
    })
}

pub fn focus(handle: isize) -> Result<(), String> {
    #[cfg(windows)]
    {
        if imp::focus(handle) {
            Ok(())
        } else {
            Err("не смогла переключиться на окно".into())
        }
    }
    #[cfg(not(windows))]
    {
        let _ = handle;
        Err("только для Windows".into())
    }
}

pub fn close(handle: isize) -> Result<(), String> {
    #[cfg(windows)]
    {
        imp::close(handle);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = handle;
        Err("только для Windows".into())
    }
}

pub fn minimize(handle: isize) -> Result<(), String> {
    #[cfg(windows)]
    {
        imp::minimize(handle);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = handle;
        Err("только для Windows".into())
    }
}

pub fn minimize_all() -> Result<(), String> {
    #[cfg(windows)]
    {
        if imp::minimize_all() {
            Ok(())
        } else {
            Err("не смогла показать рабочий стол".into())
        }
    }
    #[cfg(not(windows))]
    Err("только для Windows".into())
}

/// Полноэкранное приложение на переднем плане? (Чтобы не лезть поверх фильма.)
#[cfg(windows)]
pub fn fullscreen_active() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    let Some(fg) = foreground() else { return false };
    if matches!(fg.class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
        return false;
    }
    let Some(rect) = imp::window_rect(fg.handle) else { return false };
    unsafe {
        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
        (rect.right - rect.left) >= sw && (rect.bottom - rect.top) >= sh
    }
}

#[cfg(not(windows))]
pub fn fullscreen_active() -> bool {
    false
}

/// Адрес открытой в браузере страницы (для «запомни этот сайт»).
pub fn window_url(handle: isize) -> Option<String> {
    let script = format!(
        "$sh = New-Object -ComObject Shell.Application; \
         $w = $sh.Windows() | Where-Object {{ $_.HWND -eq {handle} }} | Select-Object -First 1; \
         if ($w) {{ $w.LocationURL }}"
    );
    let out = crate::util::ps(&script, 8).ok()?;
    let url = out.trim().to_string();
    if url.starts_with("http") {
        Some(url)
    } else {
        None
    }
}

/// Заголовок окна по хэндлу.
pub fn title_of(handle: isize) -> Option<String> {
    list()
        .into_iter()
        .find(|w| w.handle == handle)
        .map(|w| w.title)
}

/// Окно Telegram, если оно есть.
pub fn telegram_window() -> Option<WindowInfo> {
    list().into_iter().find(|w| {
        let exe = w.exe.as_deref().unwrap_or("").to_lowercase();
        exe.contains("telegram") || w.title.to_lowercase().contains("telegram")
    })
}
