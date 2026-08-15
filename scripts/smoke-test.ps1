# Smoke test: launch the built dsh-desktop.exe, wait for the host readiness
# line in the app log, then tear everything down.
param(
  [string]$Exe = '..\src-tauri\target\debug\dsh-desktop.exe'
)
$ErrorActionPreference = 'Stop'
$exe = Join-Path $PSScriptRoot $Exe
if (-not (Test-Path $exe)) { Write-Error "not found: $exe"; exit 2 }

$log = Join-Path $env:LOCALAPPDATA 'dsh-desktop\dsh-desktop.log'
New-Item -ItemType Directory -Force -Path (Split-Path $log) | Out-Null
Remove-Item $log -Force -ErrorAction SilentlyContinue

$env:DSH_DESKTOP_CWD = Join-Path $env:TEMP 'dsh-smoke'
New-Item -ItemType Directory -Force -Path $env:DSH_DESKTOP_CWD | Out-Null

$p = Start-Process -FilePath $exe -PassThru
$deadline = (Get-Date).AddSeconds(120)
$ok = $false
$failed = $false
while ((Get-Date) -lt $deadline) {
  if ($p.HasExited) {
    Write-Output "SMOKE FAIL: app exited early, code $($p.ExitCode)"
    $failed = $true
    break
  }
  if (Test-Path $log) {
    $content = Get-Content $log -Raw -ErrorAction SilentlyContinue
    if ($content -match 'host ready: http://127\.0\.0\.1:\d+') {
      Write-Output 'SMOKE OK: host ready line found'
      $ok = $true
      break
    }
    if ($content -match '\[app\] .*failed to start|\[app\] .*exited before|\[app\] .*Timed out|\[app\] .*did not report') {
      Write-Output 'SMOKE FAIL: app reported a startup failure'
      $failed = $true
      break
    }
  }
  Start-Sleep -Seconds 2
}
if (-not $ok -and -not $failed) {
  Write-Output 'SMOKE FAIL: no readiness line within 120s'
  $failed = $true
}
if ($failed -and (Test-Path $log)) {
  Write-Output '--- last log lines ---'
  Get-Content $log | Select-Object -Last 30
}

# Cleanup: stop the app, then any node host it spawned (matches only --port 0).
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Get-CimInstance Win32_Process -Filter "Name='node.exe'" |
  Where-Object { $_.CommandLine -match 'dsh' -and $_.CommandLine -match '--port 0' } |
  ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }

exit $(if ($ok) { 0 } else { 1 })
