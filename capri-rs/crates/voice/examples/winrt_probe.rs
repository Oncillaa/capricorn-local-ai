//! Проба голоса Windows 10/11 (`Windows.Media.SpeechSynthesis`).
//!
//! Запуск: `cargo run -p capri-voice --example winrt_probe`
//!
//! Печатает список голосов, синтезирует фразу, показывает размер и тип
//! полученного звука и пробует проиграть его нашим аудио.

#[cfg(windows)]
fn main() {
    use windows::Media::SpeechSynthesis::SpeechSynthesizer;
    use windows::Storage::Streams::DataReader;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

    // Без инициализации апартамента WinRT-активация отдаёт внутреннюю ошибку.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    println!("CoInitializeEx: {hr:?}");

    let synthesizer = match SpeechSynthesizer::new() {
        Ok(s) => s,
        Err(e) => {
            println!("Синтезатор не создался: {e}");
            return;
        }
    };
    println!("Синтезатор создан");

    match SpeechSynthesizer::AllVoices() {
        Ok(voices) => match voices.Size() {
            Ok(count) => {
                println!("голосов в системе: {count}");
                for index in 0..count.min(8) {
                    if let Ok(voice) = voices.GetAt(index) {
                        let name = voice.DisplayName().map(|s| s.to_string()).unwrap_or_default();
                        let language = voice.Language().map(|s| s.to_string()).unwrap_or_default();
                        println!("  {index}: {name} ({language})");
                    }
                }
            }
            Err(e) => println!("не смогла перечислить голоса: {e}"),
        },
        Err(e) => println!("AllVoices: {e}"),
    }

    let text = windows::core::HSTRING::from("Привет. Это проверка голоса Капри.");
    let operation = match synthesizer.SynthesizeTextToStreamAsync(&text) {
        Ok(operation) => operation,
        Err(e) => {
            println!("синтез не запустился: {e}");
            return;
        }
    };
    let stream = match operation.join() {
        Ok(stream) => stream,
        Err(e) => {
            println!("синтез не удался: {e}");
            return;
        }
    };
    println!("поток получен");

    match stream.ContentType() {
        Ok(kind) => println!("тип содержимого: {kind}"),
        Err(e) => println!("ContentType: {e}"),
    }
    let size = stream.Size().unwrap_or(0);
    println!("размер: {size} байт");

    let input = match stream.GetInputStreamAt(0) {
        Ok(input) => input,
        Err(e) => {
            println!("GetInputStreamAt: {e}");
            return;
        }
    };
    let reader = match DataReader::CreateDataReader(&input) {
        Ok(reader) => reader,
        Err(e) => {
            println!("DataReader: {e}");
            return;
        }
    };
    if let Err(e) = reader.LoadAsync(size as u32).and_then(|op| op.join()) {
        println!("чтение потока: {e}");
        return;
    }
    let mut bytes = vec![0u8; size as usize];
    if let Err(e) = reader.ReadBytes(&mut bytes) {
        println!("ReadBytes: {e}");
        return;
    }
    println!("прочитано {} байт, первые: {:02X?}", bytes.len(), &bytes[..4.min(bytes.len())]);

    // Пробуем проиграть тем же путём, что и обычный голос.
    match capri_voice::audio::decode_bytes(&bytes, capri_voice::audio::FX_SR) {
        Ok(samples) => {
            println!("декодировано {} сэмплов", samples.len());
            let played = capri_voice::audio::play_samples(samples, capri_voice::audio::FX_SR, 80);
            println!("проиграно (true = перебили): {played}");
        }
        Err(e) => println!("декодер не взял: {e}"),
    }
}

#[cfg(not(windows))]
fn main() {
    println!("Только для Windows");
}
