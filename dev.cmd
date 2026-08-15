@echo off
setlocal
cd /d "%~dp0"
title DeepSeek Harness Desktop - Dev

where node >nul 2>nul || (echo [ERROR] Node.js not found in PATH & goto :fail)
where cargo >nul 2>nul || (echo [ERROR] Rust toolchain not found. Install via https://rustup.rs & goto :fail)

echo Rendering black-whale app icons from whale.svg (Edge headless)...
powershell -NoProfile -ExecutionPolicy Bypass -File "scripts\generate-icons.ps1"
if errorlevel 1 goto :fail

if not exist "node_modules\.bin\tauri.cmd" (
  call npm install
  if errorlevel 1 goto :fail
)

echo Starting debug build + app. First run compiles ~400 crates (10-30 min).
call npx tauri dev
if errorlevel 1 goto :fail
exit /b 0

:fail
echo.
echo Aborted. Fix the error above and rerun.
pause
exit /b 1
