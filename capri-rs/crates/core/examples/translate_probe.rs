//! Проверка перевода: `cargo run -p capri-core --example translate_probe -- <каталог>`
//!
//! Берёт русский текст и переводит его через тот провайдер, который реально
//! настроен (LibreTranslate → локальная модель → облако).

use capri_core::translate::Translator;
use capri_core::Config;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "..".to_string());
    let path = std::path::Path::new(&dir).join("config.json");
    let cfg = Config::load(&path);

    let cache = capri_core::translate::cache_dir(std::path::Path::new(&dir));
    let translator = Translator::new(&cfg, Some(cache));
    println!("провайдер: {}", translator.backend_label());
    println!("перевожу на: {}", cfg.translate_to);

    for (text, target) in [
        ("Обведи текст мышью — переведу его.", "en"),
        ("The quick brown fox jumps over the lazy dog.", "ru"),
    ] {
        match translator.translate(text, None, target) {
            Ok(done) => println!(
                "\n{} → {}\n  было:  {text}\n  стало: {}",
                capri_core::translate::language(done.from).name,
                capri_core::translate::language(done.to).name,
                done.text
            ),
            Err(why) => println!("\nне вышло: {why}"),
        }
    }
}
