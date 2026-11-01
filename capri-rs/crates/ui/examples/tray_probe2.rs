//! РџСЂРѕР±Р° С‚СЂРµСЏ Р±РµР· РѕР±С‘СЂС‚РѕРє: `cargo run -p capri-ui --example tray_probe2`
//!
//! Р РѕРІРЅРѕ С‚РѕС‚ Р¶Рµ РІС‹Р·РѕРІ, С‡С‚Рѕ РІ `tray_probe`, РЅРѕ `NOTIFYICONDATAW` РѕР±СЉСЏРІР»РµРЅР° Р·РґРµСЃСЊ
//! СЂСѓРєР°РјРё, Р° `Shell_NotifyIconW` РІС‹Р·С‹РІР°РµС‚СЃСЏ РЅР°РїСЂСЏРјСѓСЋ. РўР°Рє РІРёРґРЅРѕ, РІРёРЅРѕРІР°С‚Р° Р»Рё
//! СЂР°СЃРєР»Р°РґРєР° СЃС‚СЂСѓРєС‚СѓСЂС‹ РёР· `windows-rs` РёР»Рё СЃР°РјР° Windows.

use std::ffi::c_void;

use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

#[repr(C)]
#[derive(Clone, Copy)]
struct Nid {
    cb_size: u32,
    hwnd: *mut c_void,
    uid: u32,
    flags: u32,
    callback: u32,
    hicon: *mut c_void,
    tip: [u16; 128],
    state: u32,
    state_mask: u32,
    info: [u16; 256],
    version: u32,
    info_title: [u16; 64],
    info_flags: u32,
    guid: [u8; 16],
    balloon: *mut c_void,
}

#[link(name = "shell32")]
extern "system" {
    fn Shell_NotifyIconW(msg: u32, data: *const Nid) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn LoadIconW(instance: *mut c_void, name: *const u16) -> *mut c_void;
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: *mut c_void,
        menu: *mut c_void,
        instance: *mut c_void,
        param: *mut c_void,
    ) -> *mut c_void;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn main() {
    unsafe {
        // РџСЂРѕРІРµСЂРєР° РµС‰С‘ РѕРґРЅРѕР№ РґРѕРіР°РґРєРё: С‡Р°СЃС‚СЊ РѕР±РѕР»РѕС‡РЅС‹С… РІС‹Р·РѕРІРѕРІ С‚СЂРµР±СѓРµС‚, С‡С‚РѕР±С‹ РІ
        // РїРѕС‚РѕРєРµ Р±С‹Р» РёРЅРёС†РёР°Р»РёР·РёСЂРѕРІР°РЅ COM. WinForms СЌС‚Рѕ РґРµР»Р°РµС‚ Р·Р° РЅР°СЃ, Р° Сѓ РЅР°СЃ
        // РґРѕ РІС‹Р·РѕРІР° РЅРёС‡РµРіРѕ РЅРµ РёРЅРёС†РёР°Р»РёР·РёСЂРѕРІР°РЅРѕ.
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        println!("COM: {hr:?}");

        let class = wide("Static");
        let title = wide("CapriTrayProbe2");
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            0x00CF_0000, // WS_OVERLAPPEDWINDOW
            10,
            10,
            200,
            120,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        println!("РѕРєРЅРѕ: {hwnd:?}");
        if hwnd.is_null() {
            println!("РѕРєРЅРѕ РЅРµ СЃРѕР·РґР°Р»РѕСЃСЊ");
            return;
        }

        // IDI_APPLICATION вЂ” СЌС‚Рѕ MAKEINTRESOURCE(32512).
        let icon = LoadIconW(std::ptr::null_mut(), 32512 as *const u16);
        println!("РёРєРѕРЅРєР°: {icon:?}");

        let mut data = Nid {
            cb_size: std::mem::size_of::<Nid>() as u32,
            hwnd,
            uid: 7,
            flags: 1 | 2 | 4,
            callback: 0x8001,
            hicon: icon,
            tip: [0; 128],
            state: 0,
            state_mask: 0,
            info: [0; 256],
            version: 0,
            info_title: [0; 64],
            info_flags: 0,
            guid: [0; 16],
            balloon: std::ptr::null_mut(),
        };
        let tip = wide("РїСЂРѕРІРµСЂРєР°");
        for (slot, code) in data.tip.iter_mut().zip(tip.iter()) {
            *slot = *code;
        }

        println!("СЂР°Р·РјРµСЂ СЃС‚СЂСѓРєС‚СѓСЂС‹: {}", data.cb_size);

        // РџРµСЂРµР±РёСЂР°РµРј РІР°СЂРёР°РЅС‚С‹ РїРѕ РѕРґРЅРѕРјСѓ: С‚Р°Рє РІРёРґРЅРѕ, С‡С‚Рѕ РёРјРµРЅРЅРѕ РЅРµ РЅСЂР°РІРёС‚СЃСЏ
        // РѕР±Р»Р°СЃС‚Рё СѓРІРµРґРѕРјР»РµРЅРёР№ вЂ” РЅР°Р±РѕСЂ С„Р»Р°РіРѕРІ, С‚РµРєСЃС‚ РїРѕРґСЃРєР°Р·РєРё РёР»Рё РІСЃС‘ СЃСЂР°Р·Сѓ.
        let variants: [(&str, u32, &str, u32); 4] = [
            ("С‚РѕР»СЊРєРѕ РїРѕРґСЃРєР°Р·РєР°, Р»Р°С‚РёРЅРёС†Р°", 4, "probe", 11),
            ("С‚РѕР»СЊРєРѕ РёРєРѕРЅРєР°", 2, "", 12),
            ("РІСЃС‘, Р»Р°С‚РёРЅРёС†Р°", 1 | 2 | 4, "probe", 13),
            ("РІСЃС‘, РєРёСЂРёР»Р»РёС†Р°", 1 | 2 | 4, "РїСЂРѕРІРµСЂРєР°", 14),
        ];
        for (name, flags, tip_text, uid) in variants {
            data.flags = flags;
            data.uid = uid;
            data.tip = [0; 128];
            let text = wide(tip_text);
            for (slot, code) in data.tip.iter_mut().zip(text.iter()) {
                *slot = *code;
            }
            let ok = Shell_NotifyIconW(0, &data) != 0;
            println!(
                "  {name}: {}",
                if ok { "РїРѕСЃС‚Р°РІР»РµРЅР°" } else { "РѕС‚РєР°Р·" }
            );
            if ok {
                let _ = Shell_NotifyIconW(2, &data);
            }
        }
    }
}

