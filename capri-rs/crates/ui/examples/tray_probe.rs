//! РџСЂРѕР±Р° С‚СЂРµСЏ: `cargo run -p capri-ui --example tray_probe`
//!
//! РЎС‚Р°РІРёС‚ РёРєРѕРЅРєСѓ РІ РѕР±Р»Р°СЃС‚СЊ СѓРІРµРґРѕРјР»РµРЅРёР№ РёР· РїСЂРѕСЃС‚РѕРіРѕ РѕРєРЅР° Рё РїРµС‡Р°С‚Р°РµС‚ СЂРµР·СѓР»СЊС‚Р°С‚.
//! РќСѓР¶РЅР°, С‡С‚РѕР±С‹ РѕС‚РґРµР»РёС‚СЊ В«РЅРµ СЂР°Р±РѕС‚Р°РµС‚ Сѓ РљР°РїСЂРёВ» РѕС‚ В«РЅРµ СЂР°Р±РѕС‚Р°РµС‚ РЅР° РјР°С€РёРЅРµВ»:
//! С‚РѕС‚ Р¶Рµ РІС‹Р·РѕРІ РёР· C# РЅР° СЌС‚РѕР№ РјР°С€РёРЅРµ РѕС‚РІРµС‡Р°РµС‚ СѓСЃРїРµС…РѕРј, Р·РЅР°С‡РёС‚ РїСЂРёС‡РёРЅР° РІ РЅР°С€РёС…
//! РїР°СЂР°РјРµС‚СЂР°С…, Рё РёСЃРєР°С‚СЊ РµС‘ РЅР°РґРѕ Р·РґРµСЃСЊ, Р° РЅРµ РІ РїРѕР»РёС‚РёРєР°С… Windows.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, LoadIconW,
    PeekMessageW, RegisterClassW, ShowWindow, TranslateMessage, MSG, PM_REMOVE,
    SW_SHOW, CS_HREDRAW, CS_VREDRAW, IDI_APPLICATION, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn main() {
    unsafe {
        let instance = match GetModuleHandleW(None) {
            Ok(handle) => handle,
            Err(why) => {
                println!("РЅРµС‚ РјРѕРґСѓР»СЏ: {why}");
                return;
            }
        };

        let class_name = wide("CapriTrayProbe");
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: HINSTANCE(instance.0),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            println!("РєР»Р°СЃСЃ РЅРµ Р·Р°СЂРµРіРёСЃС‚СЂРёСЂРѕРІР°Р»СЃСЏ: {}", GetLastError().0);
        }

        let title = wide("CapriTrayProbe");
        let hwnd = match CreateWindowExW(
            Default::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            200,
            120,
            None,
            None,
            Some(HINSTANCE(instance.0)),
            None,
        ) {
            Ok(handle) => handle,
            Err(why) => {
                println!("РѕРєРЅРѕ РЅРµ СЃРѕР·РґР°Р»РѕСЃСЊ: {why}");
                return;
            }
        };
        println!("РѕРєРЅРѕ: {hwnd:?}");
        let _ = ShowWindow(hwnd, SW_SHOW);
        println!("РѕРєРЅРѕ РїРѕРєР°Р·Р°РЅРѕ");

        // Р“РёРїРѕС‚РµР·Р°: РѕР±Р»Р°СЃС‚СЊ СѓРІРµРґРѕРјР»РµРЅРёР№ РѕС‚РєР°Р·С‹РІР°РµС‚ РѕРєРЅСѓ, С‡РµР№ РїРѕС‚РѕРє РЅРё СЂР°Р·Сѓ РЅРµ
        // СЂР°Р·Р±РёСЂР°Р» РѕС‡РµСЂРµРґСЊ СЃРѕРѕР±С‰РµРЅРёР№. WinForms РІ C# РєСЂСѓС‚РёС‚ С†РёРєР» РІСЃРµРіРґР°, Р° РјС‹
        // РґРѕ СЌС‚РѕРіРѕ РјРµСЃС‚Р° РµРіРѕ РЅРµ Р·Р°РїСѓСЃРєР°Р»Рё вЂ” РІРѕС‚ Рё СЂР°Р·РЅРёС†Р°.
        let mut msg = MSG::default();
        let until = std::time::Instant::now() + std::time::Duration::from_millis(400);
        while std::time::Instant::now() < until {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        println!("РѕС‡РµСЂРµРґСЊ СЃРѕРѕР±С‰РµРЅРёР№ СЂР°Р·РѕР±СЂР°РЅР°");

        let icon = LoadIconW(None, IDI_APPLICATION).unwrap_or_default();
        println!("РёРєРѕРЅРєР°: {icon:?}");

        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 7,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: 0x8001,
            hIcon: icon,
            ..Default::default()
        };
        let tip = wide("РїСЂРѕРІРµСЂРєР°");
        let len = tip.len().min(data.szTip.len());
        data.szTip[..len].copy_from_slice(&tip[..len]);

        let added = Shell_NotifyIconW(NIM_ADD, &data).as_bool();
        println!(
            "РїРѕСЃС‚Р°РЅРѕРІРєР°: {} (РєРѕРґ {}, cbSize={})",
            if added { "РРљРћРќРљРђ РџРћРЎРўРђР’Р›Р•РќРђ" } else { "РѕС‚РєР°Р·" },
            GetLastError().0,
            data.cbSize
        );

        if added {
            println!("РёРєРѕРЅРєР° РІРёСЃРёС‚ 20 СЃРµРєСѓРЅРґ вЂ” РїРѕСЃРјРѕС‚СЂРё РІ С‚СЂРµР№");
            std::thread::sleep(std::time::Duration::from_secs(20));
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }

        let _ = DestroyWindow(hwnd);
    }
}


