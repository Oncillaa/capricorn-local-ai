@echo off
chcp 65001 >nul
rem Проверка и сборка всего проекта на Rust.

cd /d "%~dp0\capri-rs"

where cargo >nul 2>nul
if errorlevel 1 (
    echo Rust не найден. Поставь его с https://rustup.rs и запусти снова.
    pause
    exit /b 1
)

echo === Проверка ===
cargo check --workspace
if errorlevel 1 goto fail

echo.
echo === Тесты ===
cargo test --workspace
if errorlevel 1 goto fail

echo.
echo === Сборка релиза ===
cargo build --release
if errorlevel 1 goto fail

rem Кладём готовый exe в корень, рядом с config.json.
copy /y "target\release\capri.exe" "..\capri.exe" >nul
if errorlevel 1 goto fail

echo.
echo Готово. Программа: capri.exe в корне проекта.
pause
exit /b 0

:fail
echo.
echo Что-то не сошлось — смотри ошибки выше.
pause
exit /b 1
