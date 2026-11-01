//! Голос Windows через отдельный процесс PowerShell.
//!
//! Почему так: на этой машине `ISpVoice`/`ISpeechVoice` изнутри нашего процесса
//! падает на `Speak` (`0x80045040`), хотя тот же самый SAPI, вызванный из
//! PowerShell, пишет нормальный WAV (проверено: 3.65 с живой речи). Проверка
//! ProgID показала те же CLSID, разные флаги и режимы не помогли — значит дело
//! в окружении процесса, а не в вызове.
//!
//! Поэтому запасной путь такой: просим PowerShell синтезировать фразу в файл,
//! читаем готовый WAV и играем сами. Медленнее, чем COM, но работает — и это
//! лучше, чем молчание. На нормальных машинах первым делом пробуется обычный
//! COM-путь, а этот включается только когда он не смог.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Скрыть окно консоли у дочернего процесса.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Скрипт синтеза. Лежит рядом с программой, пишется при первом обращении.
///
/// Ровно та последовательность вызовов, которая проверена в PowerShell:
/// `SpVoice` + `SpFileStream` + `Speak`. Ничего лишнего — ни выбора голоса,
/// ни громкости: громкость мы ставим своим проигрыванием.
const SCRIPT: &str = r#"param([string]$Text, [string]$Out)
$ErrorActionPreference = 'Stop'
try {
    $text = [System.IO.File]::ReadAllText($Text, [System.Text.Encoding]::UTF8)
    if ($text.Trim().Length -eq 0) { exit 2 }
    if (Test-Path $Out) { Remove-Item $Out -Force }
    $voice = New-Object -ComObject SAPI.SpVoice
    $stream = New-Object -ComObject SAPI.SpFileStream
    $stream.Open($Out, 3, $false)
    $voice.AudioOutputStream = $stream
    [void]$voice.Speak($text, 0)
    $stream.Close()
    exit 0
} catch {
    Write-Error ("SAPI: " + $_.Exception.Message + " HRESULT=0x" + ('{0:X8}' -f $_.Exception.HResult))
    exit 1
}
"#;

/// Где лежат рабочие файлы: первый каталог, куда реально можно писать.
fn work_dir(candidates: &[PathBuf]) -> Result<PathBuf, String> {
    for dir in candidates {
        if std::fs::create_dir_all(dir).is_ok() {
            return Ok(dir.clone());
        }
    }
    Err("некуда писать запись для голоса Windows".into())
}

fn ensure_script(dir: &Path) -> Result<PathBuf, String> {
    let path = dir.join("capri-tts.ps1");
    let need = match std::fs::read_to_string(&path) {
        Ok(existing) => existing != SCRIPT,
        Err(_) => true,
    };
    if need {
        std::fs::write(&path, SCRIPT).map_err(|e| format!("скрипт голоса не записался: {e}"))?;
    }
    Ok(path)
}

/// Синтезировать фразу в WAV через PowerShell. Возвращает байты WAV.
pub fn render(text: &str, candidates: &[PathBuf]) -> Result<Vec<u8>, String> {
    if text.trim().is_empty() {
        return Err("пустая фраза".into());
    }
    let dir = work_dir(candidates)?;
    let script = ensure_script(&dir)?;
    let text_path = dir.join("capri-tts-text.txt");
    let wav_path = dir.join("capri-tts-out.wav");
    std::fs::write(&text_path, text).map_err(|e| format!("текст для голоса не записался: {e}"))?;
    let _ = std::fs::remove_file(&wav_path);

    let mut command = Command::new("powershell");
    let err_path = dir.join("capri-tts-err.txt");
    let stderr = std::fs::File::create(&err_path).ok();
    command
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(&script)
        .arg("-Text")
        .arg(&text_path)
        .arg("-Out")
        .arg(&wav_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    match stderr {
        Some(file) => {
            command.stderr(file);
        }
        None => {
            command.stderr(Stdio::null());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if std::env::var("CAPRI_PS_TTS_WINDOW").is_ok() {
            // Отладка: запуск с окном консоли (вдруг дело в нём).
        } else {
            command.creation_flags(CREATE_NO_WINDOW);
        }
    }

    let status = command
        .status()
        .map_err(|e| format!("PowerShell для голоса не запустился: {e}"))?;
    if !status.success() {
        let why = std::fs::read_to_string(&err_path).unwrap_or_default();
        let why = why.trim();
        return Err(if why.is_empty() {
            format!(
                "PowerShell не озвучил фразу (код {})",
                status.code().unwrap_or(-1)
            )
        } else {
            why.to_string()
        });
    }
    let bytes = std::fs::read(&wav_path).map_err(|e| format!("запись голоса не прочиталась: {e}"))?;
    let _ = std::fs::remove_file(&text_path);
    if bytes.len() < 100 {
        return Err("запись голоса пустая".into());
    }
    Ok(bytes)
}
