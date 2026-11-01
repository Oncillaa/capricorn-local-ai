//! Р“РґРµ Р¶РёРІС‘С‚ РїСЂРѕС†РµСЃСЃ: `cargo run -p capri-ui --example tray_probe3`
//!
//! РџРµС‡Р°С‚Р°РµС‚ РёРјСЏ СЂР°Р±РѕС‡РµР№ СЃС‚Р°РЅС†РёРё, РёРјСЏ СЂР°Р±РѕС‡РµРіРѕ СЃС‚РѕР»Р° Рё РЅР°С…РѕРґРёС‚ Р»Рё РїСЂРѕС†РµСЃСЃ
//! `Shell_TrayWnd`. Р•СЃР»Рё Rust-РїСЂРѕС†РµСЃСЃ РЅРµ РІРёРґРёС‚ РїР°РЅРµР»СЊ Р·Р°РґР°С‡, С‚Рѕ Рё РѕР±Р»Р°СЃС‚СЊ
//! СѓРІРµРґРѕРјР»РµРЅРёР№ РґР»СЏ РЅРµРіРѕ РЅРµ СЃСѓС‰РµСЃС‚РІСѓРµС‚ вЂ” РѕС‚СЃСЋРґР° Рё РѕС‚РєР°Р· СЃ РєРѕРґРѕРј 5.

use windows::core::PCWSTR;
use windows::Win32::System::StationsAndDesktops::{
    GetProcessWindowStation, GetThreadDesktop, GetUserObjectInformationW, UOI_NAME,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

fn name_of(handle: windows::Win32::Foundation::HANDLE, what: &str) {
    let mut buffer = [0u16; 256];
    let mut needed = 0u32;
    let ok = unsafe {
        GetUserObjectInformationW(
            handle,
            UOI_NAME,
            Some(buffer.as_mut_ptr() as *mut _),
            (buffer.len() * 2) as u32,
            Some(&mut needed),
        )
    };
    if ok.is_err() {
        println!("{what}: РёРјСЏ РЅРµ РїРѕР»СѓС‡РµРЅРѕ");
        return;
    }
    let len = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    println!("{what}: {}", String::from_utf16_lossy(&buffer[..len]));
}

fn main() {
    unsafe {
        let station = match GetProcessWindowStation() {
            Ok(handle) => windows::Win32::Foundation::HANDLE(handle.0),
            Err(why) => {
                println!("рабочая станция: не получена ({why})");
                return;
            }
        };
        name_of(
            windows::Win32::Foundation::HANDLE(station.0),
            "СЂР°Р±РѕС‡Р°СЏ СЃС‚Р°РЅС†РёСЏ",
        );

        let thread = GetCurrentThreadId();
        let desk = GetThreadDesktop(thread);
        match desk {
            Ok(handle) => name_of(windows::Win32::Foundation::HANDLE(handle.0), "СЂР°Р±РѕС‡РёР№ СЃС‚РѕР»"),
            Err(why) => println!("СЂР°Р±РѕС‡РёР№ СЃС‚РѕР»: РЅРµ РїРѕР»СѓС‡РµРЅ ({why})"),
        }

        let class = "Shell_TrayWnd".encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let tray = FindWindowW(PCWSTR(class.as_ptr()), PCWSTR::null());
        match tray {
            Ok(handle) if !handle.is_invalid() => println!("РїР°РЅРµР»СЊ Р·Р°РґР°С‡ РІРёРґРЅР°: {handle:?}"),
            _ => println!("РїР°РЅРµР»СЊ Р·Р°РґР°С‡ РќР• РЅР°Р№РґРµРЅР°"),
        }
    }
}
