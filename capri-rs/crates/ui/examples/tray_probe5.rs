//! Проверка «а не в процессе ли дело»: `cargo run -p capri-ui --example tray_probe5 <путь к .ps1>`
//!
//! Запускает C#-проверку трея как своего потомка. Если потомок Rust-процесса
//! ставит иконку, а сам Rust-процесс нет — значит область уведомлений
//! отказывает именно нашему образу, а не дереву процессов.

use std::process::Command;

fn main() {
    let script = match std::env::args().nth(1) {
        Some(path) => path,
        None => {
            println!("нужен путь к _tray_child.ps1");
            return;
        }
    };
    match Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &script])
        .output()
    {
        Ok(out) => {
            println!("потомок Rust-процесса говорит:");
            println!("{}", String::from_utf8_lossy(&out.stdout).trim());
            let err = String::from_utf8_lossy(&out.stderr);
            if !err.trim().is_empty() {
                println!("ошибки: {}", err.trim());
            }
        }
        Err(why) => println!("не запустилось: {why}"),
    }
}
