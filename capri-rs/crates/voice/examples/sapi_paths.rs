//! РџСЂРѕР±Р°: РєР°Рє SAPI РѕС‚РєСЂС‹РІР°РµС‚ С„Р°Р№Р» вЂ” РѕС‚РЅРѕСЃРёС‚РµР»СЊРЅС‹Р№ Рё Р°Р±СЃРѕР»СЋС‚РЅС‹Р№ РїСѓС‚СЊ.
//!
//! Р—Р°РїСѓСЃРє: `cargo run -p capri-voice --example sapi_paths`

#[cfg(windows)]
fn main() {
    use windows::core::{GUID, HSTRING};
    use windows::Win32::Foundation::VARIANT_FALSE;
    use windows::Win32::Media::Speech::{ISpeechFileStream, SSFMCreateForWrite};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };

    const CLSID_SPFILESTREAM: GUID = GUID::from_u128(0x947812B3_2AE1_4644_BA86_9E90DED7EC91);

    let paths = [
        "C:\\Users\\Oncilla\\sapi-home.wav".to_string(),
        "C:\\Users\\Oncilla\\AppData\\Local\\Capri\\sapi-lcl.wav".to_string(),
        "C:\\Users\\Oncilla\\AppData\\Local\\Temp\\sapi-tmp.wav".to_string(),
        "C:\\Users\\Oncilla\\Desktop\\sapi-desk.wav".to_string(),
        "C:\\Users\\Oncilla\\Desktop\\capricorn-main\\sapi-root.wav".to_string(),
    ];

    unsafe {
        println!("CoInitializeEx: {:?}", CoInitializeEx(None, COINIT_APARTMENTTHREADED));
        for path in &paths {
            let _ = std::fs::remove_file(path);
            let stream: ISpeechFileStream =
                match CoCreateInstance(&CLSID_SPFILESTREAM, None, CLSCTX_ALL) {
                    Ok(stream) => stream,
                    Err(e) => {
                        println!("{path}: РїРѕС‚РѕРє РЅРµ СЃРѕР·РґР°Р»СЃСЏ ({e})");
                        continue;
                    }
                };
            match stream.Open(&windows::core::BSTR::from(HSTRING::from(path).to_string()), SSFMCreateForWrite, VARIANT_FALSE) {
                Ok(()) => {
                    let _ = stream.Close();
                    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                    println!("{path}: РѕРє, С„Р°Р№Р» {size} Р±Р°Р№С‚");
                }
                Err(e) => println!("{path}: РѕС€РёР±РєР° {e}"),
            }
        }
    }
}

#[cfg(not(windows))]
fn main() {
    println!("РўРѕР»СЊРєРѕ РґР»СЏ Windows");
}

