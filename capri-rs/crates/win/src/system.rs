//! Процессы, питание, диагностика, буфер обмена, Telegram.

use capri_core::platform::{PcReport, PowerMode, ProcInfo};

use crate::util::{ps, spawn_detached};

/// Список процессов (имя, pid, загрузка ЦП, память).
pub fn processes() -> Vec<ProcInfo> {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};
    let Ok(mut sys) = System::new_with_specifics(
        RefreshKind::nothing()
            .with_processes(ProcessRefreshKind::nothing().with_cpu().with_memory()),
    ) else {
        return Vec::new();
    };
    // Два замера подряд — иначе загрузка ЦП всегда 0.
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    std::thread::sleep(std::time::Duration::from_millis(300));
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    sys.processes()
        .values()
        .map(|p| ProcInfo {
            pid: p.pid().as_u32(),
            name: p.name().to_string_lossy().to_string(),
            cpu: p.cpu_usage(),
            mem_mb: p.memory() as f32 / 1_048_576.0,
        })
        .collect()
}

/// Завершить процессы по имени. Возвращает, сколько удалось.
pub fn kill_process(name: &str, force: bool) -> usize {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};
    let needle = name.to_lowercase().replace(".exe", "");
    let Ok(mut sys) =
        System::new_with_specifics(RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()))
    else {
        return 0;
    };
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let mut killed = 0usize;
    for p in sys.processes().values() {
        let pname = p.name().to_string_lossy().to_lowercase();
        let stem = pname.trim_end_matches(".exe");
        if stem == needle || stem.contains(&needle) {
            let ok = if force {
                p.kill_with(sysinfo::Signal::Kill).unwrap_or(false)
                    || std::process::Command::new("taskkill")
                        .args(["/F", "/PID", &p.pid().as_u32().to_string()])
                        .creation_flags_no_window()
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false)
            } else {
                p.kill()
            };
            if ok {
                killed += 1;
            }
        }
    }
    if killed == 0 {
        // Может быть, это окно, а не процесс — пробуем taskkill по имени.
        let _ = spawn_detached("taskkill", &["/IM", &format!("{name}.exe"), "/F"]);
    }
    killed
}

trait NoWindow {
    fn creation_flags_no_window(&mut self) -> &mut Self;
}

impl NoWindow for std::process::Command {
    fn creation_flags_no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            self.creation_flags(crate::util::CREATE_NO_WINDOW);
        }
        self
    }
}

/// Заблокировать компьютер.
pub fn lock_workstation() -> Result<(), String> {
    spawn_detached("rundll32.exe", &["user32.dll,LockWorkStation"])
        .map_err(|e| format!("не смогла заблокировать: {e}"))
}

/// Выключение / перезагрузка / сон.
pub fn power(mode: PowerMode) -> Result<(), String> {
    match mode {
        PowerMode::Shutdown => spawn_detached("shutdown", &["/s", "/t", "10"])
            .map_err(|e| format!("не смогла выключить: {e}")),
        PowerMode::Restart => spawn_detached("shutdown", &["/r", "/t", "10"])
            .map_err(|e| format!("не смогла перезагрузить: {e}")),
        PowerMode::Logoff => spawn_detached("shutdown", &["/l"])
            .map_err(|e| format!("не смогла завершить сеанс: {e}")),
        PowerMode::Sleep => {
            let script = "Add-Type -AssemblyName System.Windows.Forms; \
                          [System.Windows.Forms.Application]::SetSuspendState('Suspend', $false, $false)";
            ps(script, 10).map(|_| ()).map_err(|e| format!("не смогла усыпить: {e}"))
        }
        PowerMode::Hibernate => spawn_detached("shutdown", &["/h"])
            .map_err(|e| format!("не смогла усыпить: {e}")),
    }
}

// ---------------------------------------------------------------- буфер обмена

pub fn clipboard_get() -> Option<String> {
    let mut cb = arboard::Clipboard::new().ok()?;
    cb.get_text().ok()
}

pub fn clipboard_set(text: &str) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("буфер обмена: {e}"))?;
    cb.set_text(text.to_string())
        .map_err(|e| format!("буфер обмена: {e}"))
}

// ---------------------------------------------------------------- диагностика

/// Полная диагностика компьютера + текстовый отчёт в `reports/`.
pub fn analyze_pc(base_dir: &std::path::Path) -> PcReport {
    use sysinfo::System;

    let mut report = PcReport::default();

    let Some(mut sys) = System::new_all().ok() else {
        report.problems.push("не смогла прочитать данные о системе".into());
        return report;
    };
    sys.refresh_all();
    std::thread::sleep(std::time::Duration::from_millis(300));
    sys.refresh_cpu_usage();

    let cpu_load = sys.global_cpu_usage();
    report.cpu_load = cpu_load;
    report.ram_total_gb = sys.total_memory() as f32 / 1_073_741_824.0;
    report.ram_used_gb = sys.used_memory() as f32 / 1_073_741_824.0;
    report.uptime_hours = System::uptime().unwrap_or(0) as f32 / 3600.0;

    // Названия железа — через WMI (быстро и без прав администратора).
    if let Ok(out) = ps(
        "(Get-CimInstance Win32_Processor | Select-Object -First 1).Name; \
         (Get-CimInstance Win32_VideoController | Select-Object -First 1).Name",
        15,
    ) {
        let mut lines = out.lines().map(|l| l.trim().to_string());
        report.cpu_name = lines.next().unwrap_or_default();
        report.gpu_name = lines.next().unwrap_or_default();
    }

    // Диски.
    if let Ok(disks) = sysinfo::Disks::new_with_refreshed_list() {
        for disk in disks.iter() {
            let total = disk.total_space() as f32 / 1_073_741_824.0;
            let free = disk.available_space() as f32 / 1_073_741_824.0;
            if total > 0.1 {
                report
                    .disks
                    .push((disk.mount_point().to_string_lossy().to_string(), free, total));
            }
        }
    }

    // Проблемы и советы.
    let ram_pct = if report.ram_total_gb > 0.0 {
        report.ram_used_gb / report.ram_total_gb * 100.0
    } else {
        0.0
    };
    if ram_pct > 92.0 {
        report
            .problems
            .push(format!("оперативная память занята на {ram_pct:.0}%"));
        report
            .advice
            .push("закрой лишние вкладки браузера и тяжёлые программы".into());
    }
    if cpu_load > 90.0 {
        report.problems.push(format!("процессор загружен на {cpu_load:.0}%"));
        report.advice.push("посмотри, что грузит компьютер: «что грузит компьютер»".into());
    }
    for (name, free, total) in &report.disks {
        let pct_free = free / total * 100.0;
        if pct_free < 8.0 {
            report
                .problems
                .push(format!("на диске {name} осталось {free:.1} ГБ"));
            report.advice.push(format!("освободи место на {name}"));
        }
    }
    if System::uptime().unwrap_or(0) > 60 * 60 * 24 * 10 {
        report
            .problems
            .push("компьютер не перезагружался больше десяти дней".into());
        report.advice.push("перезагрузи компьютер — часть проблем уйдёт сама".into());
    }

    // Отчёт на диск.
    let dir = base_dir.join("reports");
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join(format!("report_{}.txt", crate::screenshot::stamp()));
    let mut text = String::from("Капри — отчёт о компьютере\n==========================\n\n");
    text.push_str(&format!("Процессор: {} (загрузка {:.0}%)\n", report.cpu_name, report.cpu_load));
    text.push_str(&format!(
        "Память: {:.1} из {:.1} ГБ ({:.0}%)\n",
        report.ram_used_gb, report.ram_total_gb, ram_pct
    ));
    text.push_str(&format!("Видеокарта: {}\n", report.gpu_name));
    text.push_str(&format!("Время работы: {:.1} ч\n\n", report.uptime_hours));
    text.push_str("Диски:\n");
    for (name, free, total) in &report.disks {
        text.push_str(&format!("  {name} — свободно {free:.1} из {total:.1} ГБ\n"));
    }
    text.push_str("\nПроблемы:\n");
    if report.problems.is_empty() {
        text.push_str("  не найдено\n");
    } else {
        for p in &report.problems {
            text.push_str(&format!("  • {p}\n"));
        }
    }
    text.push_str("\nСоветы:\n");
    if report.advice.is_empty() {
        text.push_str("  всё в порядке\n");
    } else {
        for a in &report.advice {
            text.push_str(&format!("  • {a}\n"));
        }
    }
    let _ = std::fs::write(&file, text);

    report
}

/// Топ процессов по процессору и по памяти.
pub fn top_processes(count: usize) -> (Vec<ProcInfo>, Vec<ProcInfo>) {
    let mut procs = processes();
    let mut by_cpu = procs.clone();
    by_cpu.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(std::cmp::Ordering::Equal));
    procs.sort_by(|a, b| b.mem_mb.partial_cmp(&a.mem_mb).unwrap_or(std::cmp::Ordering::Equal));
    (by_cpu.into_iter().take(count).collect(), procs.into_iter().take(count).collect())
}

// ---------------------------------------------------------------- Telegram

/// Открыть чат в Telegram Desktop и напечатать сообщение.
pub fn telegram_send(chat: &str, text: &str, press_enter: bool) -> Result<(), String> {
    use crate::input;
    use crate::windows_api;

    let window = match windows_api::telegram_window() {
        Some(w) => w,
        None => {
            crate::launch::launch_smart("telegram")?;
            std::thread::sleep(std::time::Duration::from_secs(6));
            windows_api::telegram_window().ok_or_else(|| "Telegram не запустился".to_string())?
        }
    };

    windows_api::focus(window.handle)?;
    std::thread::sleep(std::time::Duration::from_millis(400));

    // Поиск в Telegram Desktop — Ctrl+F или просто набор текста.
    input::send_keys(&[
        capri_core::platform::KeyStroke::new(capri_core::keys::VK_CTRL),
        capri_core::platform::KeyStroke::new('F' as u16),
    ])?;
    std::thread::sleep(std::time::Duration::from_millis(350));
    input::type_text(chat)?;
    std::thread::sleep(std::time::Duration::from_millis(900));
    input::send_keys(&[capri_core::platform::KeyStroke::new(capri_core::keys::VK_ENTER)])?;
    std::thread::sleep(std::time::Duration::from_millis(700));
    input::type_text(text)?;
    if press_enter {
        std::thread::sleep(std::time::Duration::from_millis(250));
        input::send_keys(&[capri_core::platform::KeyStroke::new(capri_core::keys::VK_ENTER)])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn processes_are_visible() {
        let list = super::processes();
        assert!(!list.is_empty(), "на Windows всегда есть процессы");
    }
}
