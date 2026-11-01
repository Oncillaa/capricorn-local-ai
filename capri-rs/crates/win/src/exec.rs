//! Запуск процесса с построчной отдачей вывода.
//!
//! Отличие от [`crate::util::ps`]: тот ждёт конца команды и возвращает всё
//! разом, а здесь строки уходят наверх по мере появления — иначе длинный
//! `npm install` выглядел бы как зависшее окно.
//!
//! Оба потока (stdout и stderr) читаются каждый в своей нити: если читать их
//! по очереди, процесс упрётся в заполненный буфер канала и встанет.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use capri_core::exec::{self, LineKind};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Что приходит из запущенного процесса.
#[derive(Debug, Clone)]
pub enum Event {
    /// Очередная строка вывода (stdout или stderr — для человека они равны).
    Line { text: String, kind: &'static str, err: bool },
    /// Процесс закончился.
    Done { code: Option<i32>, killed: bool },
}

/// Превратить строку вывода в событие.
fn line_event(text: impl Into<String>, err: bool) -> Event {
    let text = text.into();
    let kind = exec::classify(&text).as_str();
    Event::Line { text, kind, err }
}

/// Читать поток построчно и отправлять строки наверх.
fn spawn_reader(
    pipe: Box<dyn std::io::Read + Send>,
    is_err: bool,
    events: Sender<Event>,
    shown: Arc<std::sync::atomic::AtomicUsize>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let reader = BufReader::new(pipe);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let seen = shown.fetch_add(1, Ordering::Relaxed);
            if let Some(note) = exec::over_limit(seen) {
                let _ = events.send(line_event(note, true));
            }
            if seen > exec::MAX_LINES {
                // Ленту не заливаем, но чтение продолжаем: иначе процесс
                // встанет, упёршись в буфер канала.
                continue;
            }
            let _ = events.send(line_event(line, is_err));
        }
    })
}

/// Запустить команду PowerShell и стримить вывод.
///
/// `cancel` — флаг «останови»: его выставляет интерфейс, а мы проверяем между
/// строками и убиваем процесс. Возвращает код возврата (`None` — если убили).
pub fn run_stream(
    command: &str,
    events: Sender<Event>,
    cancel: Arc<AtomicBool>,
    timeout_secs: u64,
) -> Option<i32> {
    let text = command.trim();
    if text.is_empty() {
        let _ = events.send(Event::Done { code: None, killed: false });
        return None;
    }

    // Кодировку ставим явно: без неё русский вывод PowerShell приходит в CP866.
    let full = format!(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; \
         $ErrorActionPreference='Continue'; {text}"
    );

    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &full])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    cmd.creation_flags(crate::util::CREATE_NO_WINDOW);

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(why) => {
            let _ = events.send(Event::Line {
                text: format!("не удалось запустить: {why}"),
                kind: LineKind::Fail.as_str(),
                err: true,
            });
            let _ = events.send(Event::Done { code: None, killed: false });
            return None;
        }
    };

    // Счётчик строк общий для двух нитей: лимит на весь вывод, а не на поток.
    let shown = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut readers = Vec::new();

    if let Some(out) = child.stdout.take() {
        readers.push(spawn_reader(Box::new(out), false, events.clone(), Arc::clone(&shown)));
    }
    if let Some(err) = child.stderr.take() {
        readers.push(spawn_reader(Box::new(err), true, events.clone(), Arc::clone(&shown)));
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let mut killed = false;
    let mut code = None;

    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            killed = true;
            let _ = child.wait();
            break;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                code = status.code();
                break;
            }
            Ok(None) => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    killed = true;
                    let _ = child.wait();
                    let _ = events.send(Event::Line {
                        text: format!("остановлено по таймауту ({timeout_secs} с)"),
                        kind: LineKind::Fail.as_str(),
                        err: true,
                    });
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(why) => {
                let _ = events.send(Event::Line {
                    text: format!("процесс потерялся: {why}"),
                    kind: LineKind::Fail.as_str(),
                    err: true,
                });
                break;
            }
        }
    }

    // Даём нитям дочитать то, что осталось в буфере: иначе последние строки
    // («6 passing») не дойдут до интерфейса.
    for handle in readers {
        let _ = handle.join();
    }

    let _ = events.send(Event::Done { code, killed });
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_becomes_a_classified_event() {
        let event = line_event("✓ returns 20 items", false);
        match event {
            Event::Line { text, kind, err } => {
                assert_eq!(text, "✓ returns 20 items");
                assert_eq!(kind, "ok");
                assert!(!err);
            }
            other => panic!("ожидалась строка, пришло {other:?}"),
        }
    }

    #[test]
    fn stderr_is_marked_as_such() {
        let event = line_event("обычный текст", true);
        match event {
            Event::Line { err, .. } => assert!(err, "поток stderr должен быть помечен"),
            other => panic!("ожидалась строка, пришло {other:?}"),
        }
    }

    /// Живая проверка: настоящая команда, настоящий PowerShell.
    ///
    /// Юнит-тестов на разбор мало — здесь проверяется именно стриминг:
    /// строки приходят по одной и последним идёт «завершено».
    #[test]
    fn a_real_command_streams_its_output() {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let code = run_stream("Write-Output 'one'; Write-Output 'two'", tx, cancel, 60);
        assert_eq!(code, Some(0), "успешная команда возвращает код 0");

        let events: Vec<Event> = rx.iter().collect();
        let lines: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                Event::Line { text, .. } => Some(text.clone()),
                Event::Done { .. } => None,
            })
            .collect();
        assert!(lines.iter().any(|l| l == "one"), "видим первую строку: {lines:?}");
        assert!(lines.iter().any(|l| l == "two"), "видим вторую строку: {lines:?}");
        assert!(
            matches!(events.last(), Some(Event::Done { code: Some(0), killed: false })),
            "последним приходит «завершено», а не строка"
        );
    }

    /// Живая проверка остановки: команда не должна дожить до конца.
    #[test]
    fn a_cancelled_command_stops_early() {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            flag.store(true, Ordering::Relaxed);
        });

        let (tx, rx) = std::sync::mpsc::channel();
        // Если бы остановка не работала, тест ждал бы эти 30 секунд.
        let code = run_stream("Start-Sleep -Seconds 30", tx, cancel, 60);
        assert_eq!(code, None, "убитый процесс кода возврата не отдаёт");

        let events: Vec<Event> = rx.iter().collect();
        assert!(
            matches!(events.last(), Some(Event::Done { killed: true, .. })),
            "остановку надо честно сообщить странице: {events:?}"
        );
    }
}
