//! Запуск программ и сайтов, поиск установленных приложений и браузеров.

use std::path::{Path, PathBuf};

use capri_core::platform::InstalledProgram;
use capri_core::util::{norm, sounds_like, stem, translit};
use once_cell::sync::Lazy;
use parking_lot::Mutex;

use crate::util::{ps, ps_json, spawn_detached, to_wide};

/// Слова, по которым в «Программах и компонентах» лежат не программы, а мусор.
const SKIP_WORDS: &[&str] = &[
    "uninstall", "удален", "удалить", "деинсталл", "readme", "help", "справка", "documentation",
    "license", "support", "поддержк", "feedback", "release notes", "website", "обновлен", "update",
    "runtime", "redistributable", "sdk", "документац",
];

fn is_junk(name: &str) -> bool {
    let n = name.to_lowercase();
    SKIP_WORDS.iter().any(|w| n.contains(w))
}

// ---------------------------------------------------------------- ShellExecute

#[cfg(windows)]
fn shell_execute(target: &str, params: Option<&str>) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let file = to_wide(target);
    let op = to_wide("open");
    let params = params.map(to_wide);
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(op.as_ptr()),
            PCWSTR(file.as_ptr()),
            params.as_ref().map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr())),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    let code = result.0 as usize;
    if code > 32 {
        Ok(())
    } else {
        Err(format!("Windows не открыла «{target}» (код {code})"))
    }
}

#[cfg(not(windows))]
fn shell_execute(_target: &str, _params: Option<&str>) -> Result<(), String> {
    Err("только для Windows".into())
}

/// Запустить что угодно: ярлык, exe, `appid:`, `shell:`, `ms-settings:`, URL.
pub fn launch_smart(target: &str) -> Result<(), String> {
    let target = target.trim();
    if target.is_empty() {
        return Err("нечего запускать".into());
    }

    // Идентификатор приложения из меню «Пуск» (UWP и обычные из Get-StartApps).
    if let Some(appid) = target.strip_prefix("appid:") {
        return spawn_detached("explorer.exe", &[&format!("shell:AppsFolder\\{appid}")]);
    }

    // Ярлык из «Пуска» / рабочий стол / exe — открываем как двойным кликом.
    if Path::new(target).exists() {
        if let Err(e) = shell_execute(target, None) {
            log::info!("ShellExecute не смог ({e}), пробую через проводник");
            return spawn_detached("explorer.exe", &[target]);
        }
        return Ok(());
    }

    // `shell:Downloads`, `ms-settings:sound` и прочие URI-схемы.
    if target.contains(':') && !target.starts_with("http") {
        return shell_execute(target, None);
    }

    // Голое имя программы: notepad, calc, wt, winword…
    match shell_execute(target, None) {
        Ok(()) => Ok(()),
        Err(e) => {
            log::info!("ShellExecute «{target}» не смог ({e}), пробую start");
            spawn_detached("cmd", &["/c", "start", "", target])
        }
    }
}

/// Открыть сайт: в названном браузере, в браузере из настроек или в обычном.
pub fn open_url(url: &str, browser_exe: Option<&Path>) -> Result<(), String> {
    if let Some(exe) = browser_exe {
        if let Ok(()) = spawn_detached(&exe.to_string_lossy(), &[url]) {
            return Ok(());
        }
    }
    shell_execute(url, None)
}

// ---------------------------------------------------------------- браузеры

/// Как браузеры называют вслух → ключ браузера.
const BROWSER_WORDS: &[(&str, &str)] = &[
    (r"(гугл )?хром\w*|chrome", "chrome"),
    (r"яндекс\w*|yandex", "yandex"),
    (r"опер\w*|opera", "opera"),
    (r"(фаер|файр|файер)фокс\w*|мозил+\w*|firefox", "firefox"),
    (r"(майкрософт )?эдж\w*|edge", "edge"),
    (r"брейв\w*|brave", "brave"),
    (r"вивальди|vivaldi", "vivaldi"),
];

pub fn browser_name(key: &str) -> &'static str {
    match key {
        "chrome" => "Google Chrome",
        "yandex" => "Яндекс Браузер",
        "opera" => "Opera",
        "firefox" => "Firefox",
        "edge" => "Edge",
        "brave" => "Brave",
        "vivaldi" => "Vivaldi",
        _ => "браузер",
    }
}

static BROWSERS: Lazy<Mutex<Option<Vec<(String, PathBuf)>>>> = Lazy::new(|| Mutex::new(None));

/// Установленные браузеры: ключ → путь к exe.
pub fn installed_browsers() -> Vec<(String, PathBuf)> {
    if let Some(cached) = BROWSERS.lock().clone() {
        return cached;
    }
    let mut found: Vec<(String, PathBuf)> = Vec::new();

    let script = r#"
$roots = @('HKCU:\SOFTWARE\Clients\StartMenuInternet',
           'HKLM:\SOFTWARE\Clients\StartMenuInternet',
           'HKLM:\SOFTWARE\WOW6432Node\Clients\StartMenuInternet')
foreach ($r in $roots) {
  Get-ChildItem -Path $r -ErrorAction SilentlyContinue | ForEach-Object {
    $cmd = (Get-ItemProperty -Path ($_.PSPath + '\shell\open\command') -ErrorAction SilentlyContinue).'(default)'
    if ($cmd) { [pscustomobject]@{ Name = $_.PSChildName; Cmd = $cmd } }
  }
}
"#;
    if let Some(serde_json::Value::Array(items)) = ps_json(script, 15) {
        for item in items {
            let name = item.get("Name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            let cmd = item.get("Cmd").and_then(|v| v.as_str()).unwrap_or("");
            let Some(exe) = exe_from_command(cmd) else { continue };
            let label = format!("{} {}", name, exe.to_lowercase());
            for (_, key) in BROWSER_WORDS {
                let hit = label.contains(key) || (*key == "edge" && label.contains("msedge"));
                if hit && !found.iter().any(|(k, _)| k == key) {
                    found.push((key.to_string(), PathBuf::from(exe.clone())));
                }
            }
        }
    }

    // Запасной вариант — известные пути.
    let fallback: [(&str, &[&str]); 7] = [
        ("chrome", &["C:/Program Files/Google/Chrome/Application/chrome.exe", "C:/Program Files (x86)/Google/Chrome/Application/chrome.exe"]),
        ("edge", &["C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe", "C:/Program Files/Microsoft/Edge/Application/msedge.exe"]),
        ("yandex", &["C:/Program Files/Yandex/YandexBrowser/Application/browser.exe"]),
        ("opera", &["C:/Program Files/Opera/opera.exe", "C:/Users/Public/AppData/Local/Programs/Opera/opera.exe"]),
        ("firefox", &["C:/Program Files/Mozilla Firefox/firefox.exe"]),
        ("brave", &["C:/Program Files/BraveSoftware/Brave-Browser/Application/brave.exe"]),
        ("vivaldi", &["C:/Program Files/Vivaldi/Application/vivaldi.exe"]),
    ];
    for (key, paths) in fallback {
        if found.iter().any(|(k, _)| k == key) {
            continue;
        }
        for p in paths {
            let path = PathBuf::from(p);
            if path.is_file() {
                found.push((key.to_string(), path));
                break;
            }
        }
    }

    *BROWSERS.lock() = Some(found.clone());
    found
}

fn exe_from_command(cmd: &str) -> Option<String> {
    let cmd = cmd.trim().trim_start_matches('"');
    let lower = cmd.to_lowercase();
    let idx = lower.find(".exe")?;
    Some(cmd[..idx + 4].trim_matches('"').to_string())
}

/// «хром» / «яндекс браузер» / путь к exe → путь к exe или None.
pub fn find_browser(name: &str) -> Option<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let as_path = PathBuf::from(name);
    if as_path.is_file() {
        return Some(as_path);
    }
    let n = norm(name);
    let key = BROWSER_WORDS.iter().find_map(|(pattern, key)| {
        let ok = regex::Regex::new(&format!(r"^(?:{pattern})(?: браузер\w*)?$|^браузер\w* (?:{pattern})$"))
            .map(|re| re.is_match(&n))
            .unwrap_or(false);
        if ok {
            Some(*key)
        } else {
            None
        }
    })?;
    installed_browsers().into_iter().find(|(k, _)| k == key).map(|(_, p)| p)
}

// ---------------------------------------------------------------- программы

static PROGRAMS: Lazy<Mutex<Option<Vec<InstalledProgram>>>> = Lazy::new(|| Mutex::new(None));

/// Установленные программы: ярлыки «Пуска»/рабочего стола + реестр + Get-StartApps.
/// Список строится один раз и кэшируется (первый вызов может занять ~1–2 с).
pub fn installed_programs() -> Vec<InstalledProgram> {
    if let Some(cached) = PROGRAMS.lock().clone() {
        return cached;
    }
    let mut out: Vec<InstalledProgram> = Vec::new();
    let mut seen: std::collections::HashSet<String> = Default::default();

    let mut push = |name: String, target: String, out: &mut Vec<InstalledProgram>| {
        let key = norm(&name);
        if key.is_empty() || is_junk(&key) || !seen.insert(key.clone()) {
            return;
        }
        out.push(InstalledProgram { name: key, target });
    };

    // 1. Ярлыки из «Пуска», на рабочем столе, в общих папках.
    let mut roots: Vec<PathBuf> = Vec::new();
    for var in ["ProgramData", "APPDATA", "PUBLIC"] {
        if let Ok(base) = std::env::var(var) {
            let base = PathBuf::from(base);
            roots.push(base.join("Microsoft/Windows/Start Menu/Programs"));
            roots.push(base.join("Desktop"));
        }
    }
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Desktop"));
        roots.push(home.join("OneDrive/Desktop"));
        roots.push(home.join("AppData/Roaming/Microsoft/Windows/Start Menu/Programs"));
    }

    for root in roots {
        let mut stack = vec![root];
        let mut budget = 4000usize;
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                if budget == 0 {
                    break;
                }
                budget -= 1;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let ext = path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                if !matches!(ext.as_str(), "lnk" | "url" | "appref-ms") {
                    continue;
                }
                let Some(stem_name) = path.file_stem().map(|s| s.to_string_lossy().to_string()) else {
                    continue;
                };
                push(stem_name, path.to_string_lossy().to_string(), &mut out);
            }
        }
    }

    // 2. Приложения из меню «Пуск» (UWP и современные) — через Get-StartApps.
    let script = "Get-StartApps | Select-Object Name, AppID";
    if let Some(serde_json::Value::Array(items)) = ps_json(script, 20) {
        for item in items {
            let name = item.get("Name").and_then(|v| v.as_str()).unwrap_or("");
            let appid = item.get("AppID").and_then(|v| v.as_str()).unwrap_or("");
            if name.is_empty() || appid.is_empty() {
                continue;
            }
            push(name.to_string(), format!("appid:{appid}"), &mut out);
        }
    }

    // 3. Реестр «Программы и компоненты» — на случай, если ярлыка нет.
    let script = r#"
$roots = @('HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall',
           'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall',
           'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall')
foreach ($r in $roots) {
  Get-ChildItem $r -ErrorAction SilentlyContinue | ForEach-Object {
    $p = Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue
    if ($p.DisplayName) {
      [pscustomobject]@{ Name = $p.DisplayName; Icon = $p.DisplayIcon; Where = $p.InstallLocation }
    }
  }
}
"#;
    if let Some(serde_json::Value::Array(items)) = ps_json(script, 25) {
        for item in items {
            let name = item.get("Name").and_then(|v| v.as_str()).unwrap_or("");
            if name.is_empty() || is_junk(name) {
                continue;
            }
            let icon = item.get("Icon").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let where_ = item
                .get("Where")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim_matches('"')
                .to_string();
            // Из «C:\Path\app.exe,0» достаём путь.
            let mut exe = icon.trim().to_string();
            if let Some(pos) = exe.rfind(',') {
                let tail = &exe[pos + 1..];
                if tail.trim().parse::<i32>().is_ok() {
                    exe = exe[..pos].to_string();
                }
            }
            let exe = exe.trim_matches('"').to_string();
            let exe_ok = exe.to_lowercase().ends_with(".exe")
                && !exe.to_lowercase().contains("unins")
                && Path::new(&exe).is_file();
            if exe_ok {
                push(name.to_string(), exe, &mut out);
                continue;
            }
            // Значок — деинсталлятор (так у Steam): ищем exe с именем программы рядом.
            if !where_.is_empty() {
                let guess_name: String = name.chars().filter(|c| c.is_alphanumeric() || *c == '-').collect();
                let guess = Path::new(&where_).join(format!("{guess_name}.exe"));
                if guess.is_file() {
                    push(name.to_string(), guess.to_string_lossy().to_string(), &mut out);
                }
            }
        }
    }

    log::debug!("Найдено приложений: {}", out.len());
    *PROGRAMS.lock() = Some(out.clone());
    out
}

/// Сбросить кэш (например, после установки новой программы).
pub fn refresh_programs() {
    *PROGRAMS.lock() = None;
    *BROWSERS.lock() = None;
}

// ---------------------------------------------------------------- папки

/// Известные папки: `pictures`, `documents`, `downloads`, `desktop`, `music`,
/// `videos`, `screenshots`, `home`, `drives`, `shell:MyComputerFolder`, …
pub fn known_folder(key: &str) -> Option<PathBuf> {
    let k = norm(key);
    let home = dirs::home_dir()?;
    let direct = match k.as_str() {
        "pictures" | "изображения" | "картинки" | "фото" => dirs::picture_dir(),
        "documents" | "документы" => dirs::document_dir(),
        "downloads" | "загрузки" | "скачанное" => dirs::download_dir(),
        "desktop" | "рабочий стол" => dirs::desktop_dir(),
        "music" | "музыка" => dirs::audio_dir(),
        "videos" | "видео" => dirs::video_dir(),
        "home" | "профиль" => Some(home.clone()),
        _ => None,
    };
    if direct.is_some() {
        return direct;
    }
    if k.starts_with("shell:") {
        // «shell:Downloads» → папка загрузок и т. п.
        let tail = k.trim_start_matches("shell:").to_lowercase();
        return match tail.as_str() {
            "downloads" => dirs::download_dir(),
            "mypictures" => dirs::picture_dir(),
            "mydocuments" => dirs::document_dir(),
            "mymusic" => dirs::audio_dir(),
            "myvideo" => dirs::video_dir(),
            "desktop" => dirs::desktop_dir(),
            "profile" => Some(home),
            _ => None,
        };
    }
    if let Some(drive) = k.strip_prefix("диск ") {
        let letter = drive.trim().chars().next()?.to_ascii_uppercase();
        let path = PathBuf::from(format!("{letter}:\\"));
        if path.is_dir() {
            return Some(path);
        }
    }
    None
}

/// Все диски.
pub fn drives() -> Vec<(char, u64, u64)> {
    let mut out = Vec::new();
    for letter in b'A'..=b'Z' {
        let path = format!("{}:\\", letter as char);
        let p = Path::new(&path);
        if !p.is_dir() {
            continue;
        }
        if let Ok(usage) = fs2_space(&path) {
            out.push((letter as char, usage.0, usage.1));
        }
    }
    out
}

/// (свободно, всего) в байтах.
fn fs2_space(path: &str) -> Result<(u64, u64), ()> {
    #[cfg(windows)]
    {
        use crate::util::to_wide;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let wide = to_wide(path);
        let mut free = 0u64;
        let mut total = 0u64;
        let ok = unsafe {
            GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free), Some(&mut total), None)
        };
        if ok.is_ok() {
            return Ok((free, total));
        }
    }
    Err(())
}

/// Ограниченный поиск папки или файла по имени в профиле пользователя.
pub fn search_file(name: &str) -> Option<PathBuf> {
    let wanted_stem = stem(&name);
    if wanted_stem.is_empty() {
        return None;
    }
    let wanted_translit = translit(&wanted_stem);
    let roots: Vec<PathBuf> = [dirs::desktop_dir(), dirs::document_dir(), dirs::download_dir(), dirs::home_dir()]
        .into_iter()
        .flatten()
        .collect();

    let started = std::time::Instant::now();
    let mut budget = 60_000usize;
    for root in roots {
        let mut stack = vec![(root, 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            if budget == 0 || started.elapsed() > std::time::Duration::from_secs(2) {
                return None;
            }
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                budget = budget.saturating_sub(1);
                let path = entry.path();
                let file_name = entry.file_name().to_string_lossy().to_lowercase();
                if file_name.starts_with('.') || file_name == "node_modules" || file_name == "appdata" {
                    continue;
                }
                let candidate_stem = stem(path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default().as_str());
                let hit = candidate_stem == wanted_stem
                    || (!wanted_translit.is_empty() && translit(&candidate_stem) == wanted_translit)
                    || sounds_like(&candidate_stem, &wanted_stem);
                if hit {
                    return Some(path);
                }
                if path.is_dir() && depth < 6 {
                    stack.push((path, depth + 1));
                }
            }
        }
    }
    None
}

/// Микрофоны (устройства записи) для меню «Микрофон».
pub fn audio_input_devices() -> Vec<String> {
    let script = "Get-CimInstance -ClassName Win32_PnPEntity -ErrorAction SilentlyContinue | \
                  Where-Object { $_.PNPClass -eq 'AudioEndpoint' -and $_.Name -notmatch 'Динамик|Speakers|Наушник|Headphone|Output' } | \
                  Select-Object -ExpandProperty Name";
    match ps(script, 8) {
        Ok(out) => out
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        Err(_) => Vec::new(),
    }
}
