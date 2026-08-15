@echo off
setlocal
cd /d "%~dp0"
title DeepSeek Harness Desktop - Build

echo === DeepSeek Harness Desktop (Tauri) build ===
echo.

where node >nul 2>nul
if errorlevel 1 (
  echo [ERROR] Node.js not found in PATH. Install from https://nodejs.org
  goto :fail
)
where npm >nul 2>nul
if errorlevel 1 (
  echo [ERROR] npm not found. Install Node.js from https://nodejs.org
  goto :fail
)
where cargo >nul 2>nul
if errorlevel 1 (
  echo [ERROR] Rust toolchain not found. Install via https://rustup.rs
  echo         ^(run rustup-init.exe, default options are fine^)
  goto :fail
)
where rustc >nul 2>nul
if errorlevel 1 (
  echo [ERROR] rustc not found. Install via https://rustup.rs
  goto :fail
)

REM MSVC C++ toolchain check (rustc finds it via vswhere, not PATH).
set HAVE_MSVC=
if exist "C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe" (
  "C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 >nul 2>nul
  if not errorlevel 1 set HAVE_MSVC=1
)
if not defined HAVE_MSVC (
  echo [WARN] MSVC C++ toolchain not detected. Tauri needs Visual Studio
  echo        Build Tools with "Desktop development with C++":
  echo        https://visualstudio.microsoft.com/visual-cpp-build-tools/
  echo        The build below will fail if it is truly missing.
)

echo [1/3] Rendering black-whale app icons from whale.svg (Edge headless)...
powershell -NoProfile -ExecutionPolicy Bypass -File "scripts\generate-icons.ps1"
if errorlevel 1 goto :fail

if not exist "node_modules\.bin\tauri.cmd" (
  echo [2/3] Installing @tauri-apps/cli...
  call npm install
  if errorlevel 1 goto :fail
)

echo [3/3] Building. First build downloads and compiles ~400 crates:
echo       allow 10-30 minutes on a typical machine.
call npx tauri build
if errorlevel 1 (
  echo.
  echo [ERROR] tauri build failed. Scroll up for details.
  goto :fail
)

echo.
echo DONE. Installer: src-tauri\target\release\bundle\nsis\*.exe
echo Debug binary (no installer): src-tauri\target\debug\dsh-desktop.exe
echo After installation, uninstall from Windows Settings or run uninstall.cmd.
pause
exit /b 0

:fail
echo.
echo Build aborted. Fix the error above and rerun.
pause
exit /b 1
