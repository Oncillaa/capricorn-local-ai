//! Мониторы: сколько их, где они и какой из них основной.
//!
//! Нужно и аватару (на каком экране жить, где гулять), и панели настроек
//! (список «Экран»). Отдаём физические пиксели — ровно то, что понимает
//! `SetWindowPos`.

use capri_core::platform::MonitorInfo;
use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};

/// Признак основного монитора в `MONITORINFO::dwFlags`.
const MONITORINFOF_PRIMARY: u32 = 0x0000_0001;

/// Все мониторы. Основной — первым, остальные слева направо.
pub fn list() -> Vec<MonitorInfo> {
    let mut found: Vec<MonitorInfo> = Vec::new();
    // Указатель на вектор живёт до конца вызова `EnumDisplayMonitors`:
    // Windows обходит мониторы синхронно и в этот же стек.
    let data = LPARAM(&mut found as *mut Vec<MonitorInfo> as isize);
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), data);
    }

    let mut monitors = found;
    // Основной — вперёд, дальше по левому краю: так список в настройках не
    // прыгает от запуска к запуску.
    monitors.sort_by_key(|m| (!m.primary, m.x, m.y));
    for (index, monitor) in monitors.iter_mut().enumerate() {
        monitor.index = index as u32;
    }
    monitors
}

unsafe extern "system" fn collect(
    monitor: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let list = &mut *(data.0 as *mut Vec<MonitorInfo>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let as_basic = &mut info.monitorInfo as *mut MONITORINFO;
    // MONITORINFOEXW начинается с MONITORINFO — так документировано, поэтому
    // приводим указатель, а не копируем структуру.
    if !GetMonitorInfoW(monitor, as_basic).as_bool() {
        return BOOL(1);
    }
    let rect = info.monitorInfo.rcMonitor;
    let name: String = info
        .szDevice
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| char::from_u32(*c as u32).unwrap_or('?'))
        .collect();
    list.push(MonitorInfo {
        index: list.len() as u32,
        name,
        x: rect.left,
        y: rect.top,
        width: (rect.right - rect.left).max(0) as u32,
        height: (rect.bottom - rect.top).max(0) as u32,
        primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
    });
    BOOL(1)
}
