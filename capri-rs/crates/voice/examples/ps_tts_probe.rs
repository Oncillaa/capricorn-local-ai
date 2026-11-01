//! Проба: синтез через PowerShell из процесса Rust.
//!
//! Запуск: `cargo run -p capri-voice --example ps_tts_probe`

use std::path::PathBuf;

fn main() {
    let dirs = vec![PathBuf::from(
        "C:\\Users\\Oncilla\\Desktop\\capricorn-main\\capri-rs\\target",
    )];
    // Окружение нашего процесса — сравним с окружением оболочки.
    let env_dump: Vec<String> = std::env::vars().map(|(k, v)| format!("{k}={v}")).collect();
    let _ = std::fs::write(
        "C:\\Users\\Oncilla\\Desktop\\capricorn-main\\capri-rs\\target\\ps_env_rust.txt",
        env_dump.join("\r\n"),
    );
    let result = match capri_voice::ps_sapi::render("Привет, это проверка голоса через PowerShell.", &dirs) {
        Ok(bytes) => format!("ок: {} байт WAV", bytes.len()),
        Err(e) => format!("ошибка: {e}"),
    };
    println!("{result}");
    // Запущенный из проводника процесс не имеет консоли — пишем результат в файл.
    let _ = std::fs::write(
        "C:\\Users\\Oncilla\\Desktop\\capricorn-main\\capri-rs\\target\\ps_tts_probe_result.txt",
        &result,
    );
}
