param(
    [string]$Exe = '..\src-tauri\target\debug\dsh-desktop.exe',
    [int]$TimeoutSeconds = 30,
    [switch]$UseBundledRuntime,
    [switch]$UseInstalledWebViewDataDirectory,
    [switch]$TestMarket,
    [switch]$AllowCandidateFallbackError,
    [string]$DshHome,
    [ValidateSet('legacy', 'core-first', 'core-crash', 'plugins-never')]
    [string]$FakeHostScenario = 'legacy',
    [int]$FakePluginDelayMs = 500
)

$ErrorActionPreference = 'Stop'

$exePath = if ([System.IO.Path]::IsPathRooted($Exe)) {
    [System.IO.Path]::GetFullPath($Exe)
}
else {
    [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot $Exe))
}
$fakeHostPath = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'fixtures\fake-host.js'))
$runtimeLock = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\runtime.lock.json') -Raw | ConvertFrom-Json
if (-not (Test-Path -LiteralPath $exePath -PathType Leaf)) {
    throw "Desktop executable not found: $exePath"
}
if (-not (Test-Path -LiteralPath $fakeHostPath -PathType Leaf)) {
    throw "Fake Host entry not found: $fakeHostPath"
}

$nodePath = if ($UseBundledRuntime) { $null } else { (Get-Command node.exe -ErrorAction Stop).Source }
$smokeRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("dsh-desktop-smoke-" + [guid]::NewGuid().ToString('N'))
$logDirectory = Join-Path $smokeRoot 'logs'
$workingDirectory = Join-Path $smokeRoot 'workspace'
$logPath = Join-Path $logDirectory 'dsh-desktop.log'
$webviewDataDirectory = Join-Path $smokeRoot 'webview-data'
New-Item -ItemType Directory -Force -Path $logDirectory, $workingDirectory, $webviewDataDirectory | Out-Null

$previousEnvironment = @{
    DSH_HOME = $env:DSH_HOME
    DSH_DESKTOP_LOG_DIR = $env:DSH_DESKTOP_LOG_DIR
    DSH_DESKTOP_NODE_EXECUTABLE = $env:DSH_DESKTOP_NODE_EXECUTABLE
    DSH_DESKTOP_CLI_ENTRY = $env:DSH_DESKTOP_CLI_ENTRY
    DSH_DESKTOP_CWD = $env:DSH_DESKTOP_CWD
    DSH_DESKTOP_USER_HOME = $env:DSH_DESKTOP_USER_HOME
    DSH_DESKTOP_READY_TIMEOUT_SECS = $env:DSH_DESKTOP_READY_TIMEOUT_SECS
    DSH_DESKTOP_CORE_READY_TIMEOUT_SECS = $env:DSH_DESKTOP_CORE_READY_TIMEOUT_SECS
    DSH_DESKTOP_PLUGIN_READY_TIMEOUT_SECS = $env:DSH_DESKTOP_PLUGIN_READY_TIMEOUT_SECS
    DSH_DESKTOP_FAKE_HOST_SCENARIO = $env:DSH_DESKTOP_FAKE_HOST_SCENARIO
    DSH_DESKTOP_FAKE_PLUGIN_DELAY_MS = $env:DSH_DESKTOP_FAKE_PLUGIN_DELAY_MS
    DSH_DESKTOP_WEBVIEW_TEST_DATA_DIR = $env:DSH_DESKTOP_WEBVIEW_TEST_DATA_DIR
}

$desktopProcess = $null
$hostProcessId = $null
$hostStartsAtReady = $null
$secondaryProcesses = @()
$succeeded = $false

# 通过 Market 自身 HTTP API 安装并卸载测试插件，覆盖 UI 实际使用的完整包管理链路。
function Invoke-MarketSmoke {
    param(
        [Parameter(Mandatory = $true)][string]$BaseUrl,
        [Parameter(Mandatory = $true)][string]$ProfileDirectory,
        [Parameter(Mandatory = $true)][int]$RequestTimeoutSeconds
    )

    $registry = Invoke-RestMethod -Uri "$BaseUrl/dsh-market/registry" -TimeoutSec 15
    $entry = $registry.registry.plugins |
        Where-Object { $_.npm -eq 'dsh-pet' -and $_.url -eq 'https://github.com/PC2005-cloud/dsh-pet/tree/main/dsh-pet' } |
        Select-Object -First 1
    if (-not $entry) {
        throw 'Market smoke could not find the locked dsh-pet registry entry.'
    }

    $headers = @{ Origin = $BaseUrl }
    try {
        $install = Invoke-RestMethod `
            -Method Post `
            -Uri "$BaseUrl/dsh-market/install" `
            -Headers $headers `
            -ContentType 'application/json' `
            -Body (@{ url = $entry.url } | ConvertTo-Json -Compress) `
            -TimeoutSec $RequestTimeoutSeconds
        if (-not $install.ok) {
            throw "Market returned an unsuccessful install result: $($install | ConvertTo-Json -Depth 8 -Compress)"
        }

        $manifest = Get-Content -LiteralPath (Join-Path $ProfileDirectory 'package.json') -Raw | ConvertFrom-Json
        if (-not $manifest.dependencies.'dsh-pet') {
            throw 'Market reported success but dsh-pet is absent from profile dependencies.'
        }

        $uninstall = Invoke-RestMethod `
            -Method Post `
            -Uri "$BaseUrl/dsh-market/uninstall" `
            -Headers $headers `
            -ContentType 'application/json' `
            -Body (@{ name = 'dsh-pet' } | ConvertTo-Json -Compress) `
            -TimeoutSec $RequestTimeoutSeconds
        if (-not $uninstall.ok) {
            throw "Market returned an unsuccessful uninstall result: $($uninstall | ConvertTo-Json -Depth 8 -Compress)"
        }

        $manifest = Get-Content -LiteralPath (Join-Path $ProfileDirectory 'package.json') -Raw | ConvertFrom-Json
        if ($manifest.dependencies.'dsh-pet') {
            throw 'Market reported successful uninstall but dsh-pet remains in profile dependencies.'
        }
    }
    catch {
        $marketLog = try {
            (Invoke-WebRequest -UseBasicParsing "$BaseUrl/dsh-market/logs" -TimeoutSec 10).Content
        }
        catch {
            'Market log endpoint was unavailable.'
        }
        throw "Market install/uninstall smoke failed: $($_.Exception.Message)`n$marketLog"
    }
}

try {
    if ($TestMarket -and -not $UseBundledRuntime) {
        throw 'Market smoke requires -UseBundledRuntime.'
    }

    # WebView2 依赖真实 Windows 用户目录；只隔离 DSH_HOME，避免伪造用户身份导致初始化阻塞。
    $env:DSH_HOME = if ([string]::IsNullOrWhiteSpace($DshHome)) {
        Join-Path $smokeRoot '.dsh'
    } else {
        [System.IO.Path]::GetFullPath($DshHome)
    }
    $env:DSH_DESKTOP_LOG_DIR = $logDirectory
    if ($UseBundledRuntime) {
        Remove-Item Env:DSH_DESKTOP_NODE_EXECUTABLE -ErrorAction SilentlyContinue
        Remove-Item Env:DSH_DESKTOP_CLI_ENTRY -ErrorAction SilentlyContinue
    }
    else {
        $env:DSH_DESKTOP_NODE_EXECUTABLE = $nodePath
        $env:DSH_DESKTOP_CLI_ENTRY = $fakeHostPath
    }
    $env:DSH_DESKTOP_CWD = $workingDirectory
    $env:DSH_DESKTOP_USER_HOME = $smokeRoot
    $env:DSH_DESKTOP_READY_TIMEOUT_SECS = [Math]::Max(10, $TimeoutSeconds).ToString()
    $env:DSH_DESKTOP_CORE_READY_TIMEOUT_SECS = [Math]::Max(10, $TimeoutSeconds).ToString()
    $env:DSH_DESKTOP_PLUGIN_READY_TIMEOUT_SECS = [Math]::Max(10, $TimeoutSeconds).ToString()
    $env:DSH_DESKTOP_FAKE_HOST_SCENARIO = $FakeHostScenario
    $env:DSH_DESKTOP_FAKE_PLUGIN_DELAY_MS = $FakePluginDelayMs.ToString()
    if ($UseInstalledWebViewDataDirectory) {
        # 安装版位于隔离临时目录，使用其默认 WebView2 目录可避免显式 data directory 在新版 WebView2 下阻塞初始化。
        Remove-Item Env:DSH_DESKTOP_WEBVIEW_TEST_DATA_DIR -ErrorAction SilentlyContinue
    }
    else {
        $env:DSH_DESKTOP_WEBVIEW_TEST_DATA_DIR = $webviewDataDirectory
    }

    if ($TestMarket) {
        # 模拟由 pnpm 10 创建的已有 profile，防止未来再次把 JSON 元数据误判为新 profile。
        $modulesDirectory = Join-Path $env:DSH_HOME 'profiles\web\node_modules'
        New-Item -ItemType Directory -Force -Path $modulesDirectory | Out-Null
        [ordered]@{
            layoutVersion = 5
            nodeLinker = 'hoisted'
            packageManager = "pnpm@$($runtimeLock.pnpm.version)"
            storeDir = Join-Path $env:LOCALAPPDATA 'pnpm\store\v10'
            virtualStoreDir = Join-Path $modulesDirectory '.pnpm'
            virtualStoreDirMaxLength = 60
        } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $modulesDirectory '.modules.yaml') -Encoding utf8NoBOM
    }

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
            $pidMatches = @([regex]::Matches($content, 'host started: pid=(\d+)'))
            if ($pidMatches.Count -gt 0) {
                $hostProcessId = [int]$pidMatches[-1].Groups[1].Value
            }
            if ($content -match 'phase=core_ready duration_ms=\d+ attempt=') {
                $hostStartsAtReady = $pidMatches.Count
                $ready = $true
                break
            }
            $unexpectedErrors = @([regex]::Matches($content, '(?m)^.*level=ERROR.*$') | ForEach-Object { $_.Value } | Where-Object {
                -not ($AllowCandidateFallbackError -and $_ -match 'runtime candidate failed validation; falling back to active runtime')
            })
            if ($unexpectedErrors.Count -gt 0) {
                throw 'Desktop logged an error before readiness.'
            }
        }
        Start-Sleep -Milliseconds 100
    }
    if (-not $ready -or -not $hostProcessId) {
        throw 'Timed out waiting for Host readiness and PID log.'
    }
    $expectedHostStarts = if ($AllowCandidateFallbackError) { 2 } else { 1 }
    if ($hostStartsAtReady -ne $expectedHostStarts) {
        throw "Unexpected Host start count at readiness: $hostStartsAtReady (expected $expectedHostStarts)."
    }
    if ($AllowCandidateFallbackError -and
        $content -notmatch 'runtime candidate failed validation; falling back to active runtime') {
        throw 'Candidate fallback smoke reached readiness without the expected rejection log.'
    }

    # WebView2 的内置启动页必须留在主 WebView；回归时这里曾错误拉起系统浏览器。
    $navigationTimeoutSeconds = [Math]::Min(60, [Math]::Max(5, $TimeoutSeconds))
    $navigationDeadline = (Get-Date).AddSeconds($navigationTimeoutSeconds)
    do {
        $content = Get-Content -LiteralPath $logPath -Raw -ErrorAction Stop
        if ($content -match 'webview_navigation decision=allow scheme=https? host=tauri\.localhost port=- path=') {
            break
        }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $navigationDeadline)
    if ($content -notmatch 'webview_navigation decision=allow scheme=https? host=tauri\.localhost port=- path=') {
        throw 'Desktop smoke did not observe the internal tauri.localhost startup navigation.'
    }
    if ($content -match 'decision=external_browser scheme=https? host=tauri\.localhost(?:\s|$)') {
        throw 'Desktop attempted to open tauri.localhost in the external browser.'
    }

    if ($TestMarket) {
        $urlMatch = [regex]::Match($content, 'dsh (?:desktop-core|web): (http://127\.0\.0\.1:\d+)')
        if (-not $urlMatch.Success) {
            throw 'Could not read the ready URL for Market smoke.'
        }
        Invoke-MarketSmoke `
            -BaseUrl $urlMatch.Groups[1].Value `
            -ProfileDirectory (Join-Path $env:DSH_HOME 'profiles\web') `
            -RequestTimeoutSeconds ([Math]::Max(120, $TimeoutSeconds))
        Write-Host 'MARKET SMOKE OK: pnpm 10 profile installed and uninstalled dsh-pet through Market.'
    }

    # 二次启动必须快速退出，并由现有实例处理聚焦，不能再创建 Host。
    $second = Start-Process -FilePath $exePath -PassThru
    $secondaryProcesses += $second
    if (-not $second.WaitForExit(10000)) {
        throw 'Secondary instance did not exit within 10 seconds.'
    }
    Start-Sleep -Milliseconds 300
    $content = Get-Content -LiteralPath $logPath -Raw
    if ([regex]::Matches($content, 'host started: pid=').Count -ne $hostStartsAtReady) {
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
    $secondaryProcesses += $quitRequest
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
    foreach ($process in $secondaryProcesses) {
        $process.Refresh()
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        }
    }
    # 唯一临时日志只属于本次 smoke，可安全回收其中记录的全部 Host PID。
    if (Test-Path -LiteralPath $logPath -PathType Leaf) {
        $recordedHostIds = [regex]::Matches(
            (Get-Content -LiteralPath $logPath -Raw),
            'host started: pid=(\d+)'
        ) | ForEach-Object { [int]$_.Groups[1].Value } | Sort-Object -Unique
        foreach ($recordedHostId in $recordedHostIds) {
            if (Get-Process -Id $recordedHostId -ErrorAction SilentlyContinue) {
                Stop-Process -Id $recordedHostId -Force -ErrorAction SilentlyContinue
            }
        }
    }

    foreach ($name in $previousEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process')
    }
    if (-not $succeeded) {
        Write-Warning "Smoke diagnostics retained at: $smokeRoot"
    }
    elseif (Test-Path -LiteralPath $smokeRoot) {
        $systemTemp = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\') + '\'
        $resolvedSmokeRoot = [System.IO.Path]::GetFullPath($smokeRoot)
        if (-not $resolvedSmokeRoot.StartsWith($systemTemp, [System.StringComparison]::OrdinalIgnoreCase) -or
            -not ([System.IO.Path]::GetFileName($resolvedSmokeRoot)).StartsWith('dsh-desktop-smoke-')) {
            throw "Refusing to delete an unexpected smoke directory: $resolvedSmokeRoot"
        }

        $cleanupDeadline = (Get-Date).AddSeconds(15)
        do {
            try {
                Remove-Item -LiteralPath $resolvedSmokeRoot -Recurse -Force -ErrorAction Stop
            }
            catch {
                if ((Get-Date) -ge $cleanupDeadline) {
                    throw
                }
                Start-Sleep -Milliseconds 250
            }
        } while (Test-Path -LiteralPath $resolvedSmokeRoot)
    }
}

if (-not $succeeded) {
    exit 1
}
