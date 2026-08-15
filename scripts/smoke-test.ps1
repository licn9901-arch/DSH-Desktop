param(
    [string]$Exe = '..\src-tauri\target\debug\dsh-desktop.exe',
    [int]$TimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'

$exePath = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot $Exe))
$fakeHostPath = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'fixtures\fake-host.js'))
if (-not (Test-Path -LiteralPath $exePath -PathType Leaf)) {
    throw "Desktop executable not found: $exePath"
}
if (-not (Test-Path -LiteralPath $fakeHostPath -PathType Leaf)) {
    throw "Fake Host entry not found: $fakeHostPath"
}

$nodePath = (Get-Command node.exe -ErrorAction Stop).Source
$smokeRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("dsh-desktop-smoke-" + [guid]::NewGuid().ToString('N'))
$localAppData = Join-Path $smokeRoot 'localappdata'
$workingDirectory = Join-Path $smokeRoot 'workspace'
$logPath = Join-Path $localAppData 'dsh-desktop\dsh-desktop.log'
New-Item -ItemType Directory -Force -Path $localAppData, $workingDirectory | Out-Null

$previousEnvironment = @{
    LOCALAPPDATA = $env:LOCALAPPDATA
    DSH_DESKTOP_NODE_EXECUTABLE = $env:DSH_DESKTOP_NODE_EXECUTABLE
    DSH_DESKTOP_CLI_ENTRY = $env:DSH_DESKTOP_CLI_ENTRY
    DSH_DESKTOP_CWD = $env:DSH_DESKTOP_CWD
    DSH_DESKTOP_READY_TIMEOUT_SECS = $env:DSH_DESKTOP_READY_TIMEOUT_SECS
}

$desktopProcess = $null
$hostProcessId = $null
$succeeded = $false

try {
    $env:LOCALAPPDATA = $localAppData
    $env:DSH_DESKTOP_NODE_EXECUTABLE = $nodePath
    $env:DSH_DESKTOP_CLI_ENTRY = $fakeHostPath
    $env:DSH_DESKTOP_CWD = $workingDirectory
    $env:DSH_DESKTOP_READY_TIMEOUT_SECS = '10'

    $desktopProcess = Start-Process -FilePath $exePath -PassThru
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $ready = $false
    while ((Get-Date) -lt $deadline) {
        $desktopProcess.Refresh()
        if ($desktopProcess.HasExited) {
            throw "Desktop exited before readiness with code $($desktopProcess.ExitCode)."
        }
        if (Test-Path -LiteralPath $logPath) {
            $content = Get-Content -LiteralPath $logPath -Raw -ErrorAction Stop
            $pidMatch = [regex]::Match($content, 'host started: pid=(\d+)')
            if ($pidMatch.Success) {
                $hostProcessId = [int]$pidMatch.Groups[1].Value
            }
            if ($content -match 'host ready: http://127\.0\.0\.1:\d+') {
                $ready = $true
                break
            }
            if ($content -match 'level=ERROR') {
                throw 'Desktop logged an error before readiness.'
            }
        }
        Start-Sleep -Milliseconds 100
    }
    if (-not $ready -or -not $hostProcessId) {
        throw 'Timed out waiting for the fake Host readiness and PID log.'
    }

    # 二次启动必须快速退出，并由现有实例处理聚焦，不能再创建 Host。
    $second = Start-Process -FilePath $exePath -PassThru
    if (-not $second.WaitForExit(10000)) {
        throw 'Secondary instance did not exit within 10 seconds.'
    }
    Start-Sleep -Milliseconds 300
    $content = Get-Content -LiteralPath $logPath -Raw
    if ([regex]::Matches($content, 'host started: pid=').Count -ne 1) {
        throw 'Secondary launch created another Host.'
    }
    if ($content -notmatch 'secondary launch requested; focusing existing window') {
        throw 'Existing instance did not receive the secondary launch event.'
    }

    # 发送主窗口关闭消息后，进程和本次记录的 Host PID 都必须继续存活。
    $closeDeadline = (Get-Date).AddSeconds(5)
    while ($desktopProcess.MainWindowHandle -eq 0 -and (Get-Date) -lt $closeDeadline) {
        Start-Sleep -Milliseconds 100
        $desktopProcess.Refresh()
    }
    if (-not $desktopProcess.CloseMainWindow()) {
        throw 'Could not send a close request to the main window.'
    }
    Start-Sleep -Milliseconds 500
    $desktopProcess.Refresh()
    if ($desktopProcess.HasExited) {
        throw 'Desktop exited instead of hiding to tray.'
    }
    if (-not (Get-Process -Id $hostProcessId -ErrorAction SilentlyContinue)) {
        throw 'Host exited when the main window was closed.'
    }

    # 自动化退出参数进入与托盘“退出”相同的幂等清理路径。
    $quitRequest = Start-Process -FilePath $exePath -ArgumentList '--quit-existing' -PassThru
    if (-not $quitRequest.WaitForExit(10000)) {
        throw 'Quit request process did not exit within 10 seconds.'
    }
    if (-not $desktopProcess.WaitForExit(15000)) {
        throw 'Existing desktop instance did not exit within 15 seconds.'
    }
    if (Get-Process -Id $hostProcessId -ErrorAction SilentlyContinue) {
        throw "Recorded Host PID $hostProcessId remained after explicit exit."
    }

    Write-Host "SMOKE OK: ready, single instance, close-to-tray and PID $hostProcessId cleanup verified."
    $succeeded = $true
}
finally {
    # 失败兜底也只处理本次脚本记录的两个 PID，绝不扫描其他 node.exe。
    if ($desktopProcess -and -not $desktopProcess.HasExited) {
        Stop-Process -Id $desktopProcess.Id -Force -ErrorAction SilentlyContinue
    }
    if ($hostProcessId -and (Get-Process -Id $hostProcessId -ErrorAction SilentlyContinue)) {
        Stop-Process -Id $hostProcessId -Force -ErrorAction SilentlyContinue
    }

    foreach ($name in $previousEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process')
    }
    if (Test-Path -LiteralPath $smokeRoot) {
        Remove-Item -LiteralPath $smokeRoot -Recurse -Force
    }
}

if (-not $succeeded) {
    exit 1
}
