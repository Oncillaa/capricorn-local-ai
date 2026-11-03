# Установка Капри в папку пользователя и ярлык на рабочем столе.
#
# Зачем это нужно: папка проекта — рабочая папка агента, и всё, что запускается
# из неё, попадает под ограничения песочницы. Из-за них Windows не даёт
# поставить иконку в трей (Shell_NotifyIconW отвечает «отказано в доступе»).
# Установленная копия живёт в обычной папке пользователя и работает нормально.

$ErrorActionPreference = "Stop"

$source = Split-Path -Parent $PSScriptRoot
$target = Join-Path $env:LOCALAPPDATA "Capri"

New-Item -ItemType Directory -Path $target -Force | Out-Null

$exe = Join-Path $source "capri.exe"
if (-not (Test-Path $exe)) {
    Write-Output "Не нашла capri.exe рядом с папкой tools — собери проект."
    exit 1
}
Copy-Item $exe (Join-Path $target "capri.exe") -Force

# config.json не перезаписываем: в нём уже могут быть настройки пользователя.
$config = Join-Path $target "config.json"
$sourceConfig = Join-Path $source "config.json"
if (-not (Test-Path $config)) {
    if (Test-Path $sourceConfig) {
        Copy-Item $sourceConfig $config -Force
    }
}

$assets = Join-Path $source "assets"
if (Test-Path $assets) {
    Copy-Item $assets (Join-Path $target "assets") -Recurse -Force
}

# Модель распознавания речи. Без неё Капри при каждом запуске говорила «я тебя
# не слышу», и человек шёл проверять микрофон — хотя микрофон был в порядке:
# папки models в установке просто не было. 87 МБ копируем один раз, а не при
# каждой установке.
$models = Join-Path $source "models"
if (Test-Path $models) {
    $vosk = Join-Path $models "vosk-model-small-ru-0.22"
    $targetVosk = Join-Path $target "models\vosk-model-small-ru-0.22"
    if ((Test-Path $vosk) -and -not (Test-Path (Join-Path $targetVosk "conf"))) {
        Write-Output "Копирую модель распознавания речи (около 90 МБ, один раз)…"
        New-Item -ItemType Directory -Path (Join-Path $target "models") -Force | Out-Null
        Copy-Item $vosk $targetVosk -Recurse -Force
    }
    $lib = Join-Path $models "libvosk.dll"
    if (Test-Path $lib) {
        Copy-Item $lib (Join-Path $target "libvosk.dll") -Force
    }
}

# libvosk.dll лежит в корне проекта, а не в models. Без неё модель открыть
# нечем: Windows отвечает «LoadLibraryExW failed».
#
# Одной libvosk мало: она собрана под MinGW и тянет за собой libgcc_s_seh-1.dll,
# libstdc++-6.dll и libwinpthread-1.dll. Пока их не было рядом, Windows
# отказывалась грузить саму libvosk — а сообщение об этом выглядело как «нет
# модели», и человек шёл проверять исправный микрофон. Копируем весь набор.
foreach ($name in @("libvosk.dll", "libgcc_s_seh-1.dll", "libstdc++-6.dll", "libwinpthread-1.dll")) {
    $from = Join-Path $source $name
    if (Test-Path $from) {
        Copy-Item $from (Join-Path $target $name) -Force
    } else {
        Write-Output "Внимание: рядом с папкой tools нет $name — Капри не будет слышать."
    }
}

# Ярлык на рабочем столе, чтобы запускать двойным кликом, как обычную программу.
$desktop = [Environment]::GetFolderPath("Desktop")
$link = Join-Path $desktop "Капри.lnk"
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($link)
$shortcut.TargetPath = Join-Path $target "capri.exe"
$shortcut.WorkingDirectory = $target
$shortcut.Description = "Капри — локальный голосовой помощник"
$shortcut.Save()

Write-Output "Установлено в: $target"
Write-Output "Ярлык: $link"
Write-Output "Настройки: $config"
