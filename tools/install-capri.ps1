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
