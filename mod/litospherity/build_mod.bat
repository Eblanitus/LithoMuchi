@echo off
REM ================================================================
REM build_mod.bat — сборка мода litospherity для Horizon (Minecraft 26.3)
REM ================================================================
REM Переменные окружения (при необходимости скорректируйте под свой ПК):
REM   HORIZON_DIR   — папка клона Horizon (по умолчанию C:\LithoMuchi\horizon)
REM   MOD_SRC       — папка с исходниками мода (по умолчанию эта же папка)
REM ================================================================

setlocal enabledelayedexpansion

if not defined HORIZON_DIR set HORIZON_DIR=C:\LithoMuchi\horizon
if not defined MOD_SRC set MOD_SRC=%~dp0

set MOD_DST=%HORIZON_DIR%\crates\litospherity_mod

echo [litospherity] Копирование исходников мода в %MOD_DST%...
if not exist "%MOD_DST%\" mkdir "%MOD_DST%"

REM Копируем Rust-хелперы инъекций
xcopy /E /I /Y "%MOD_SRC%\mixins\litospherity\*.rs" "%MOD_DST%\src\mixins\litospherity\"
REM Копируем манифест и конфиг миксинов
xcopy /E /I /Y "%MOD_SRC%\horizon-native.json" "%MOD_DST%\"
xcopy /E /I /Y "%MOD_SRC%\mixins\litospherity\horizon-mixin.toml" "%MOD_DST%\"
REM Копируем Cargo-манифест и корневые исходники пакета
xcopy /E /I /Y "%MOD_SRC%\Cargo.toml" "%MOD_DST%\"
xcopy /E /I /Y "%MOD_SRC%\src\*.rs" "%MOD_DST%\src\"
xcopy /E /I /Y "%MOD_SRC%\src\mixins\*.rs" "%MOD_DST%\src\mixins\"
xcopy /E /I /Y "%MOD_SRC%\src\mixins\litospherity\*.rs" "%MOD_DST%\src\mixins\litospherity\"

if errorlevel 1 (
    echo [litospherity] ОШИБКА: не удалось скопировать файлы мода.
    exit /b 1
)

echo [litospherity] Сборка через cargo...
cd /d "%HORIZON_DIR%"
cargo build --release -p litospherity_mod

if errorlevel 1 (
    echo [litospherity] ОШИБКА: cargo build завершился с ошибкой.
    exit /b 1
)

echo [litospherity] Сборка завершена успешно.
pause
