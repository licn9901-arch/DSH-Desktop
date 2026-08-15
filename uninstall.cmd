@echo off
setlocal EnableExtensions
title Uninstall DeepSeek Harness Desktop
cd /d "%~dp0"

set "APP_NAME=DeepSeek Harness Desktop"
set "UNINSTALL_CMD="

REM The NSIS installer registers the app under the Windows uninstall key.
for /f "tokens=2,*" %%A in ('reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\%APP_NAME%" /v UninstallString 2^>nul ^| find /i "UninstallString"') do set "UNINSTALL_CMD=%%B"
if not defined UNINSTALL_CMD (
  for /f "tokens=2,*" %%A in ('reg query "HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\%APP_NAME%" /v UninstallString 2^>nul ^| find /i "UninstallString"') do set "UNINSTALL_CMD=%%B"
)

REM Fallback for a current-user install made by Tauri's default NSIS mode.
if not defined UNINSTALL_CMD (
  if exist "%LOCALAPPDATA%\%APP_NAME%\uninstall.exe" (
    start "" "%LOCALAPPDATA%\%APP_NAME%\uninstall.exe"
    exit /b 0
  )
)

REM Fallback for a per-machine install.
if not defined UNINSTALL_CMD (
  if exist "C:\Program Files\%APP_NAME%\uninstall.exe" (
    start "" "C:\Program Files\%APP_NAME%\uninstall.exe"
    exit /b 0
  )
)

if not defined UNINSTALL_CMD (
  echo [ERROR] DeepSeek Harness Desktop is not installed, or its uninstaller
  echo         was not found in the registry.
  echo.
  echo         Install it first with the NSIS setup from:
  echo         src-tauri\target\release\bundle\nsis\*.exe
  echo.
  pause
  exit /b 1
)

echo Starting: %UNINSTALL_CMD%
start "" %UNINSTALL_CMD%
exit /b 0
