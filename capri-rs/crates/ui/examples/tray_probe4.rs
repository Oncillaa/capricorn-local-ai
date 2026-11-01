//! РљС‚Рѕ РјС‹ РґР»СЏ СЃРёСЃС‚РµРјС‹: `cargo run -p capri-ui --example tray_probe4`
//!
//! РџРµС‡Р°С‚Р°РµС‚ СѓСЂРѕРІРµРЅСЊ С†РµР»РѕСЃС‚РЅРѕСЃС‚Рё РїСЂРѕС†РµСЃСЃР° Рё РїРµСЂРµРјРµРЅРЅС‹Рµ СЃРµР°РЅСЃР°. Р•СЃР»Рё Rust-РїСЂРѕС†РµСЃСЃ
//! РѕРєР°Р·Р°Р»СЃСЏ РІС‹С€Рµ Explorer РїРѕ СѓСЂРѕРІРЅСЋ С†РµР»РѕСЃС‚РЅРѕСЃС‚Рё, С‚Рѕ UIPI Р±Р»РѕРєРёСЂСѓРµС‚ РѕР±СЂР°С‰РµРЅРёРµ Рє
//! РѕР±Р»Р°СЃС‚Рё СѓРІРµРґРѕРјР»РµРЅРёР№ вЂ” Рё СЌС‚Рѕ СЂРѕРІРЅРѕ С‚РѕС‚ СЃР°РјС‹Р№ РѕС‚РєР°Р· СЃ РєРѕРґРѕРј 5.

use std::process::Command;

fn main() {
    // `whoami /groups` РїРµС‡Р°С‚Р°РµС‚ РјР°РЅРґР°С‚РЅС‹Р№ СѓСЂРѕРІРµРЅСЊ вЂ” С‚Рѕ Р¶Рµ, С‡С‚Рѕ РІРёРґРёС‚ UIPI.
    match Command::new("whoami").arg("/groups").output() {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                if line.contains("Mandatory") || line.contains("РћР±СЏР·Р°С‚РµР»СЊРЅС‹Р№") {
                    println!("СѓСЂРѕРІРµРЅСЊ С†РµР»РѕСЃС‚РЅРѕСЃС‚Рё: {}", line.trim());
                }
            }
        }
        Err(why) => println!("whoami РЅРµ Р·Р°РїСѓСЃС‚РёР»СЃСЏ: {why}"),
    }

    // РЎРІРѕР№ С‚РѕРєРµРЅ С†РµР»РёРєРѕРј: РёРјСЏ РїРѕР»СЊР·РѕРІР°С‚РµР»СЏ Рё РїСЂРёР·РЅР°Рє РїРѕРІС‹С€РµРЅРёСЏ РїСЂР°РІ.
    match Command::new("whoami").arg("/user").output() {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            println!("РїРѕР»СЊР·РѕРІР°С‚РµР»СЊ: {}", text.trim());
        }
        Err(why) => println!("whoami /user: {why}"),
    }

    for key in ["SESSIONNAME", "USERNAME", "USERDOMAIN"] {
        println!("{key}={}", std::env::var(key).unwrap_or_else(|_| "вЂ”".into()));
    }

    // Р’С‹СЃРѕРєРё Р»Рё РјС‹ РїРѕ С„Р°РєС‚Сѓ: OpenProcessToken + TokenElevation С‡РµСЂРµР· net user
    // РЅРµ РїСЂРѕРІРµСЂРёС‚СЊ, РїРѕСЌС‚РѕРјСѓ СЃРїСЂР°С€РёРІР°РµРј Сѓ СЃР°РјРѕР№ СЃРёСЃС‚РµРјС‹ С‡РµСЂРµР· PowerShell,
    // РєРѕС‚РѕСЂС‹Р№ СѓРјРµРµС‚ СЌС‚Рѕ С€С‚Р°С‚РЅРѕ.
    let script = "[bool](New-Object Security.Principal.WindowsPrincipal(\
        [Security.Principal.WindowsIdentity]::GetCurrent())).IsInRole(\
        [Security.Principal.WindowsBuiltInRole]::Administrator)";
    match Command::new("powershell")
        .args(["-NoProfile", "-Command", script])
        .output()
    {
        Ok(out) => println!(
            "Р°РґРјРёРЅРёСЃС‚СЂР°С‚РѕСЂ (РїРѕ РјРЅРµРЅРёСЋ РґРѕС‡РµСЂРЅРµРіРѕ PowerShell): {}",
            String::from_utf8_lossy(&out.stdout).trim()
        ),
        Err(why) => println!("PowerShell РЅРµ Р·Р°РїСѓСЃС‚РёР»СЃСЏ: {why}"),
    }
}

