//! Проверка локальной нейросети: `cargo run -p capri-core --example ai_probe`
//!
//! Печатает, что нашлось по адресам из `config.json` и по обычным портам,
//! и пробует один живой вопрос.

use capri_core::ai::LocalAi;
use capri_core::Config;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "..".to_string());
    let path = std::path::Path::new(&dir).join("config.json");
    let cfg = Config::load(&path);
    println!("адрес из настроек: {:?}", cfg.local_ai_url);
    println!("модель из настроек: {:?}", cfg.local_model);

    let ai = LocalAi::new(&cfg);
    println!("доступна: {}", ai.available());
    match ai.describe() {
        Some(where_) => println!("нашла: {where_}"),
        None => println!("нашла: ничего"),
    }
    match ai.models() {
        Some(models) => {
            println!("моделей на сервере: {}", models.len());
            for m in models.iter().take(4) {
                println!("  - {m}");
            }
        }
        None => println!("список моделей не получен"),
    }
    println!("выбранная модель: {:?}", ai.effective_model());

    match ai.ask("Ответь одним предложением: ты на связи?") {
        Some(answer) => println!("ответ: {answer}"),
        None => println!("ответ: пусто"),
    }
}
