//! Проба: как заставить SAPI говорить в этом окружении.
//!
//! Запуск: `cargo run -p capri-voice --example sapi_probe`
//!
//! Проверяем каждый голос по очереди: пишем WAV-файл и смотрим размер.
//! Устройств вывода у SAPI нет (`AudioOutput` пуст), но движок должен работать.

#[cfg(windows)]
fn main() {
    use windows::core::{w, GUID, PCWSTR};
    use windows::Win32::Media::Audio::WAVEFORMATEX;
    use windows::Win32::Media::Speech::{
        ISpObjectTokenCategory, ISpStream, ISpVoice, SPFM_CREATE_ALWAYS,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };

    const CLSID_SPVOICE: GUID = GUID::from_u128(0x96749377_3391_11D2_9EE3_00C04F797396);
    const CLSID_SPOBJECTTOKENCATEGORY: GUID =
        GUID::from_u128(0xA910187F_0C7A_45AC_92CC_59EDAFB77B53);
    const CLSID_SPSTREAM: GUID = GUID::from_u128(0x947812B3_2AE1_4644_BA86_9E90DED7EC91);
    const SPDFID_WAVEFORMATEX: GUID = GUID::from_u128(0xC31ADBAE_527F_4FF5_A230_F62BB61FF70C);

    unsafe {
        println!("CoInitializeEx: {:?}", CoInitializeEx(None, COINIT_APARTMENTTHREADED));
        let category: ISpObjectTokenCategory =
            match CoCreateInstance(&CLSID_SPOBJECTTOKENCATEGORY, None, CLSCTX_ALL) {
                Ok(category) => category,
                Err(e) => {
                    println!("категория не создалась: {e}");
                    return;
                }
            };
        if let Err(e) =
            category.SetId(w!("HKEY_LOCAL_MACHINE\\SOFTWARE\\Microsoft\\Speech\\Voices"), false)
        {
            println!("SetId(Voices) ошибка {e}");
            return;
        }
        let tokens = match category.EnumTokens(PCWSTR::null(), PCWSTR::null()) {
            Ok(tokens) => tokens,
            Err(e) => {
                println!("EnumTokens(Voices) ошибка {e}");
                return;
            }
        };
        let mut count = 0u32;
        let _ = tokens.GetCount(&mut count);
        println!("голосов у SAPI: {count}");

        for index in 0..count {
            let Ok(token) = tokens.Item(index) else { continue };
            let id = token
                .GetId()
                .map(|p| p.to_string().unwrap_or_default())
                .unwrap_or_default();
            println!("--- {index}: {id}");

            let voice: ISpVoice = match CoCreateInstance(&CLSID_SPVOICE, None, CLSCTX_ALL) {
                Ok(voice) => voice,
                Err(e) => {
                    println!("    SpVoice не создался: {e}");
                    continue;
                }
            };
            if let Err(e) = voice.SetVoice(&token) {
                println!("    SetVoice ошибка {e}");
                continue;
            }
            let path = format!("target/sapi-voice-{index}.wav");
            let _ = std::fs::remove_file(&path);
            let stream: ISpStream = match CoCreateInstance(&CLSID_SPSTREAM, None, CLSCTX_ALL) {
                Ok(stream) => stream,
                Err(e) => {
                    println!("    SpStream не создался: {e}");
                    continue;
                }
            };
            let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
            let format = WAVEFORMATEX {
                wFormatTag: 1,
                nChannels: 1,
                nSamplesPerSec: 22_050,
                nAvgBytesPerSec: 44_100,
                nBlockAlign: 2,
                wBitsPerSample: 16,
                cbSize: 0,
            };
            if let Err(e) = stream.BindToFile(
                PCWSTR(wide.as_ptr()),
                SPFM_CREATE_ALWAYS,
                Some(&SPDFID_WAVEFORMATEX),
                Some(&format),
                0,
            ) {
                println!("    BindToFile ошибка {e}");
                continue;
            }
            if let Err(e) = voice.SetOutput(&stream, true) {
                println!("    SetOutput ошибка {e}");
            }
            match voice.Speak(w!("Проверка голоса."), 0, None) {
                Ok(()) => println!("    Speak ок"),
                Err(e) => println!("    Speak ошибка {e}"),
            }
            let _ = stream.Close();
            match std::fs::metadata(&path) {
                Ok(meta) => println!("    WAV: {} байт", meta.len()),
                Err(e) => println!("    файла нет: {e}"),
            }
        }
    }
}

#[cfg(not(windows))]
fn main() {
    println!("Только для Windows");
}
