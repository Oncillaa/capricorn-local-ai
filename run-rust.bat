@echo off
chcp 65001 >nul
rem Запуск Капри 2.0 на Rust.
rem Ищет готовый capri.exe, а если его нет — собирает релиз.

cd /d "%~dp0"

rem Сначала готовый exe в корне, потом свежесобранный в capri-rs.
set "EXE=capri.exe"
if exist "%EXE%" goto run
set "EXE=capri-rs\target\release\capri.exe"
if exist "%EXE%" goto run

:build
echo Сборка (первый раз это около минуты)...
where cargo >nul 2>nul
if errorlevel 1 goto norust

pushd capri-rs
cargo build --release
if errorlevel 1 (
    popd
    echo.
    echo Сборка не удалась. Смотри ошибки выше.
    pause
    exit /b 1
)
popd

rem Кладём exe рядом с config.json, чтобы запуск был двойным кликом.
if exist "capri-rs\target\release\capri.exe" copy /y "capri-rs\target\release\capri.exe" "capri.exe" >nul
set "EXE=capri.exe"

:run
echo Запускаю Капри...
start "" "%EXE%"
exit /b 0

:norust
echo.
echo Rust не найден. Поставь его так:
echo   1. Скачай rustup-init.exe с https://rustup.rs
echo   2. Запусти и согласись со значениями по умолчанию
echo   3. Перезапусти этот файл
echo.
pause
exit /b 1
