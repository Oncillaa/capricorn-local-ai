//! Мелкие помощники: широкие строки, PowerShell, COM.

use std::process::{Command, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// `CREATE_NO_WINDOW` — чтобы не мигали чёрные окна консоли.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Выполнить PowerShell и вернуть stdout (UTF-8, обрезанный).
pub fn ps(script: &str, timeout_secs: u64) -> Result<String, String> {
    let full = format!(
        "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; \
         $ErrorActionPreference='SilentlyContinue'; {script}"
    );
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &full])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd.spawn().map_err(|e| format!("powershell: {e}"))?;

    // Простой сторож по времени: ждём в отдельных потоках чтения.
    let stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = String::new();
        if let Some(mut s) = stdout {
            let mut raw = Vec::new();
            let _ = s.read_to_end(&mut raw);
            buf = String::from_utf8_lossy(&raw).to_string();
        }
        buf
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("powershell не ответил вовремя".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
            Err(e) => return Err(format!("powershell: {e}")),
        }
    }
    let out = reader.join().unwrap_or_default();
    Ok(out.trim().to_string())
}

/// Выполнить PowerShell, который печатает JSON, и разобрать его.
pub fn ps_json(script: &str, timeout_secs: u64) -> Option<serde_json::Value> {
    let out = ps(&format!("{script} | ConvertTo-Json -Depth 4 -Compress"), timeout_secs).ok()?;
    if out.is_empty() {
        return Some(serde_json::Value::Array(Vec::new()));
    }
    serde_json::from_str(&out).ok().map(|v| match v {
        serde_json::Value::Array(_) => v,
        other => serde_json::Value::Array(vec![other]),
    })
}

/// Запустить процесс без окна консоли.
pub fn spawn_detached(program: &str, args: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.spawn().map(|_| ()).map_err(|e| format!("{program}: {e}"))
}

/// RAII-обёртка инициализации COM для текущего потока.
pub struct Com;

impl Com {
    pub fn init() -> Self {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        Com
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows::Win32::System::Com::CoUninitialize();
        }
    }
}
