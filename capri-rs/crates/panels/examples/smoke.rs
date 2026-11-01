//! Ручная проверка панелей без всей программы.
//!
//! Открывает окно настроек, ждёт `ready` от страницы и отдаёт ей поддельное
//! состояние — так видно, что работает весь путь: протокол `capri://` →
//! страница → `window.ipc.postMessage` → ядро → `evaluate_script`.
//!
//! Запуск: `cargo run -p capri-panels --example smoke`

use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

fn main() {
    let (commands, command_rx) = channel();
    let (events_tx, events) = channel();

    // Цикл панелей живёт в своём потоке — как и в самой программе.
    std::thread::spawn(move || {
        let code = capri_panels::run(command_rx, events_tx);
        println!("панели завершились, код {code}");
        std::process::exit(code);
    });

    let _ = commands.send(capri_panels::Command::Open(capri_panels::PanelId::Settings));

    let deadline = Instant::now() + Duration::from_secs(25);
    while Instant::now() < deadline {
        let Ok(event) = events.recv_timeout(Duration::from_millis(500)) else {
            continue;
        };
        println!("событие: {} / {} / {}", event.panel.as_str(), event.action, event.data);

        match event.action.as_str() {
            "ready" => {
                let state = serde_json::json!({
                    "app": "capri",
                    "version": "2.0.0-smoke",
                    "base_dir": "C:\\smoke",
                    "config_path": "C:\\smoke\\config.json",
                    "monitors": [
                        { "index": 0, "name": "\\\\.\\DISPLAY1", "width": 2560, "height": 1440,
                          "primary": true, "label": "Монитор 1 — 2560×1440 (основной)" },
                        { "index": 1, "name": "\\\\.\\DISPLAY2", "width": 1920, "height": 1080,
                          "primary": false, "label": "Монитор 2 — 1920×1080" },
                    ],
                    "config": {
                        "enable_ai": true,
                        "local_ai": true,
                        "local_model": "qwen2.5-3b-instruct",
                        "model": "claude-opus-5-5",
                        "tts_engine": "edge",
                        "voice": "ru-RU-SvetlanaNeural",
                        "voice_volume": 80,
                        "mic_device": "Микрофон (проверка)",
                        "wake_words": ["капри", "каприз"],
                        "character_scale": 55,
                        "liveliness": "calm",
                        "monitor_index": 1,
                    },
                });
                let js = format!("window.capri.setState({state});");
                let _ = commands.send(capri_panels::Command::Eval(event.panel, js));

                // Контрольный вопрос странице: стили применились, разделов
                // столько, сколько нужно, а настройки вида легли в органы
                // управления.
                let probe = r#"
                    const size = document.getElementById('size-range');
                    const liveliness = Array.from(document.querySelectorAll('#liveliness .btn'));
                    const monitor = document.getElementById('monitor');
                    window.ipc.postMessage(JSON.stringify({
                        action: 'probe',
                        data: {
                            bg: getComputedStyle(document.body).backgroundColor,
                            nav: document.querySelectorAll('.nav-item').length,
                            facts: document.querySelectorAll('#facts dd').length,
                            size: document.getElementById('size-value').textContent,
                            slider: size.value,
                            sliderFill: size.style.getPropertyValue('--fill'),
                            liveliness: liveliness.filter((b) => b.classList.contains('is-on')).map((b) => b.textContent),
                            monitors: Array.from(monitor.options).map((o) => o.text),
                            monitorValue: monitor.value,
                            title: document.title,
                        },
                    }));
                "#;
                let _ = commands.send(capri_panels::Command::Eval(event.panel, probe.to_string()));

                // А теперь — как будто пользователь потрогал органы управления:
                // жмём «Спокойная» и двигаем ползунок размера. В ответ должны
                // прийти события `set-liveliness` и (после паузы) `set-scale`.
                let interact = r#"
                    setTimeout(() => {
                        document.querySelector('[data-liveliness="calm"]').click();
                        const slider = document.getElementById('size-range');
                        slider.value = '35';
                        slider.dispatchEvent(new Event('input', { bubbles: true }));
                    }, 150);
                "#;
                let _ = commands.send(capri_panels::Command::Eval(event.panel, interact.to_string()));
            }
            "close" => break,
            _ => {}
        }
    }

    let _ = commands.send(capri_panels::Command::Quit);
    std::thread::sleep(Duration::from_millis(400));
}
