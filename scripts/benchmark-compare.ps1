param(
    [Parameter(Mandatory = $true)][string]$LegacyInstaller,
    [Parameter(Mandatory = $true)][string]$PayloadInstaller,
    [int]$WarmPairs = 20,
    [int]$ColdRuns = 3,
    [int]$TimeoutSeconds = 180,
    [string]$SeedProfile,
    [string]$Output
)

$ErrorActionPreference = 'Stop'
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
. (Join-Path $PSScriptRoot 'release-installer-isolation.ps1')
. (Join-Path $PSScriptRoot 'release-source.ps1')
$sourceCommit = Get-DshReleaseSourceCommit -RepoRoot $repoRoot
$package = Get-Content -LiteralPath (Join-Path $repoRoot 'package.json') -Raw | ConvertFrom-Json
$defaultOutput = Join-Path $repoRoot ".release-work\$($package.version)\reports\startup-comparison.json"
$reportPath = if ([string]::IsNullOrWhiteSpace($Output)) {
    $defaultOutput
} elseif ([System.IO.Path]::IsPathRooted($Output)) {
    [System.IO.Path]::GetFullPath($Output)
} else {
    [System.IO.Path]::GetFullPath((Join-Path $repoRoot $Output))
}
$releaseRoot = [System.IO.Path]::GetFullPath((Join-Path $repoRoot ".release-work\$($package.version)"))
$releasePrefix = $releaseRoot.TrimEnd('\') + '\'
if (-not $reportPath.StartsWith($releasePrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Startup comparison report must stay under $releaseRoot"
}
if ($WarmPairs -ne 20) { throw 'Release startup gate requires exactly 20 warm pairs.' }
if ($ColdRuns -ne 3) { throw 'Release startup report requires exactly 3 cold runs per build.' }

# 将仓库相对路径解析为已存在的安装器文件。
function Resolve-InstallerPath {
    param([Parameter(Mandatory = $true)][string]$Path)
    $resolved = if ([System.IO.Path]::IsPathRooted($Path)) {
        [System.IO.Path]::GetFullPath($Path)
    } else {
        [System.IO.Path]::GetFullPath((Join-Path $repoRoot $Path))
    }
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw "Installer not found: $resolved"
    }
    return $resolved
}

# 复制相同 seed profile，确保两个安装版只因自身启动路径产生差异。
function Copy-SeedProfile {
    param(
        [Parameter(Mandatory = $true)][string]$Destination,
        [string]$Source
    )
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    if (-not [string]::IsNullOrWhiteSpace($Source)) {
        $sourcePath = [System.IO.Path]::GetFullPath($Source)
        if (-not (Test-Path -LiteralPath $sourcePath -PathType Container)) {
            throw "Seed profile not found: $sourcePath"
        }
        foreach ($entry in Get-ChildItem -LiteralPath $sourcePath -Force) {
            Copy-Item -LiteralPath $entry.FullName -Destination $Destination -Recurse -Force
        }
    }
}

# 为一个安装版设置隔离环境；子进程只会继承当前临时根。
function Set-BenchmarkEnvironment {
    param([Parameter(Mandatory = $true)]$Context)
    $env:DSH_HOME = $Context.DshHome
    $env:LOCALAPPDATA = $Context.LocalAppData
    $env:DSH_DESKTOP_CWD = $Context.Workspace
    $env:DSH_DESKTOP_LOG_DIR = $Context.LogDirectory
    $env:DSH_DESKTOP_USER_HOME = $Context.UserHome
    $env:DSH_DESKTOP_READY_TIMEOUT_SECS = $TimeoutSeconds.ToString()
    $env:DSH_DESKTOP_CORE_READY_TIMEOUT_SECS = $TimeoutSeconds.ToString()
    $env:DSH_DESKTOP_PLUGIN_READY_TIMEOUT_SECS = $TimeoutSeconds.ToString()
    # 安装目录本身已隔离；默认 WebView2 数据目录会随临时安装目录一起清理。
    Remove-Item Env:DSH_DESKTOP_WEBVIEW_TEST_DATA_DIR -ErrorAction SilentlyContinue
}

# 返回当前隔离安装目录拥有的桌面进程，禁止按名称清理其他安装实例。
function Get-OwnedDesktopProcesses {
    param([Parameter(Mandatory = $true)]$Context)
    $expected = [System.IO.Path]::GetFullPath($Context.Exe)
    return @(Get-Process dsh-desktop -ErrorAction SilentlyContinue | Where-Object {
        $_.Path -and [System.IO.Path]::GetFullPath($_.Path).Equals(
            $expected,
            [System.StringComparison]::OrdinalIgnoreCase
        )
    })
}

# 静默安装到当前隔离目录，并等待 NSIS 后台阶段与 payload provision 完成。
function Install-BenchmarkBuild {
    param(
        [Parameter(Mandatory = $true)]$Context,
        [Parameter(Mandatory = $true)][string]$Installer,
        [Parameter(Mandatory = $true)][bool]$Payload
    )
    Set-BenchmarkEnvironment $Context
    $arguments = @('/S', '/NS')
    if ($Payload) {
        $arguments += "/PAYLOADTESTROOT=$($Context.RuntimeRoot)"
    } else {
        $arguments += "/DSHHOME=$($Context.DshHome)"
    }
    $arguments += "/D=$($Context.InstallRoot)"
    $installTimeoutSeconds = if ($Payload) { $TimeoutSeconds } else { [Math]::Max($TimeoutSeconds, 900) }
    $process = Start-Process -FilePath $Installer -ArgumentList $arguments -PassThru
    if (-not $process.WaitForExit($installTimeoutSeconds * 1000)) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        throw "Installer timed out after $installTimeoutSeconds seconds: $Installer"
    }
    if ($process.ExitCode -ne 0) { throw "Installer failed with code $($process.ExitCode): $Installer" }

    $deadline = (Get-Date).AddSeconds($installTimeoutSeconds)
    do {
        $ready = (Test-Path -LiteralPath $Context.Exe -PathType Leaf) -and
            (Test-Path -LiteralPath $Context.Uninstaller -PathType Leaf)
        if ($Payload) {
            $ready = $ready -and (Test-Path -LiteralPath (Join-Path $Context.RuntimeRoot 'runtime-state.json') -PathType Leaf)
        }
        if (-not $ready) { Start-Sleep -Milliseconds 250 }
    } while (-not $ready -and (Get-Date) -lt $deadline)
    if (-not $ready) { throw "Installed build did not become ready: $($Context.InstallRoot)" }
}

# 启动一次安装版，用外部墙钟测量到 core 与 plugins 均 ready，并通过正式单实例命令退出。
function Invoke-BenchmarkRun {
    param(
        [Parameter(Mandatory = $true)]$Context,
        [Parameter(Mandatory = $true)][string]$Label
    )
    Set-BenchmarkEnvironment $Context
    if ((Get-OwnedDesktopProcesses $Context).Count -gt 0) {
        throw "Benchmark instance is already running: $($Context.Exe)"
    }
    New-Item -ItemType Directory -Force -Path $Context.LogDirectory, $Context.Workspace | Out-Null
    $logPath = Join-Path $Context.LogDirectory 'dsh-desktop.log'
    if (Test-Path -LiteralPath $logPath) { Remove-Item -LiteralPath $logPath -Force }
    $startupWatch = [System.Diagnostics.Stopwatch]::StartNew()
    $process = Start-Process -FilePath $Context.Exe -PassThru
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $duration = $null
    $content = ''
    do {
        Start-Sleep -Milliseconds 100
        $process.Refresh()
        if ($process.HasExited) { throw "$Label exited before full readiness with code $($process.ExitCode)." }
        if (Test-Path -LiteralPath $logPath -PathType Leaf) {
            $content = Get-Content -LiteralPath $logPath -Raw
            if ($content -match 'host ready: https?://[^\s]+ \(started in \d+ ms\)') {
                $duration = [int]$startupWatch.ElapsedMilliseconds
            }
        }
    } while ($null -eq $duration -and (Get-Date) -lt $deadline)
    if ($null -eq $duration) { throw "$Label timed out waiting for full readiness." }

    $quit = Start-Process -FilePath $Context.Exe -ArgumentList '--quit-existing' -PassThru
    if (-not $quit.WaitForExit(10000)) {
        Stop-Process -Id $quit.Id -Force -ErrorAction SilentlyContinue
        throw "$Label quit request timed out."
    }
    if (-not $process.WaitForExit(15000)) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        throw "$Label desktop did not exit after quit request."
    }
    $hostIds = [regex]::Matches($content, 'host started: pid=(\d+)') |
        ForEach-Object { [int]$_.Groups[1].Value } | Sort-Object -Unique
    foreach ($hostId in $hostIds) {
        if (Get-Process -Id $hostId -ErrorAction SilentlyContinue) {
            Stop-Process -Id $hostId -Force -ErrorAction SilentlyContinue
            throw "$Label left Host PID $hostId running."
        }
    }
    return $duration
}

# 按最近秩计算发布报告使用的百分位数。
function Get-Percentile {
    param(
        [Parameter(Mandatory = $true)][int[]]$Samples,
        [Parameter(Mandatory = $true)][double]$Percentile
    )
    $sorted = @($Samples | Sort-Object)
    return $sorted[[Math]::Ceiling($sorted.Count * $Percentile) - 1]
}

# 静默卸载当前隔离安装版；失败时保留诊断根供人工复核。
function Uninstall-BenchmarkBuild {
    param([Parameter(Mandatory = $true)]$Context)
    if (-not (Test-Path -LiteralPath $Context.Uninstaller -PathType Leaf)) { return }
    Set-BenchmarkEnvironment $Context
    $process = Start-Process -FilePath $Context.Uninstaller -ArgumentList '/S' -PassThru
    if (-not $process.WaitForExit(60000)) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        throw "Uninstaller timed out: $($Context.Uninstaller)"
    }
    if ($process.ExitCode -ne 0) { throw "Uninstaller failed with code $($process.ExitCode)" }
}

$legacyPath = Resolve-InstallerPath $LegacyInstaller
$payloadPath = Resolve-InstallerPath $PayloadInstaller
$expectedLegacyHash = 'e331e628b07bf574e823610324130c258d77ed1e57113b59426feed1a3a9d3d9'
$actualLegacyHash = (Get-FileHash -LiteralPath $legacyPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualLegacyHash -ne $expectedLegacyHash) {
    throw "Legacy preview.7 baseline SHA-256 mismatch: $actualLegacyHash"
}
$payloadHash = (Get-FileHash -LiteralPath $payloadPath -Algorithm SHA256).Hash.ToLowerInvariant()
Assert-DshInstallerTestUserIsClean
$seedPath = if ([string]::IsNullOrWhiteSpace($SeedProfile)) { $null } else { [System.IO.Path]::GetFullPath($SeedProfile) }
$contexts = @{}
foreach ($name in @('legacy', 'payload')) {
    $contextRoot = New-DshInstallerTestRoot
    $installRoot = Join-Path $contextRoot 'install'
    $contexts[$name] = [pscustomobject]@{
        Name = $name
        Root = $contextRoot
        InstallRoot = $installRoot
        Exe = Join-Path $installRoot 'dsh-desktop.exe'
        Uninstaller = Join-Path $installRoot 'uninstall.exe'
        DshHome = Join-Path $contextRoot '.dsh'
        LocalAppData = Join-Path $contextRoot 'localappdata'
        RuntimeRoot = Join-Path $contextRoot 'localappdata\dsh-desktop\runtime'
        Workspace = Join-Path $contextRoot 'workspace'
        LogDirectory = Join-Path $contextRoot 'logs'
        UserHome = Join-Path $contextRoot 'user-home'
        WebViewData = Join-Path $contextRoot 'webview-data'
    }
    Copy-SeedProfile -Destination $contexts[$name].DshHome -Source $seedPath
    New-Item -ItemType Directory -Force -Path $contexts[$name].WebViewData | Out-Null
}
Assert-DshInstallerTestRoots -OwnedInstallRoots @($contexts.legacy.InstallRoot, $contexts.payload.InstallRoot)

$environmentNames = @(
    'DSH_HOME', 'LOCALAPPDATA', 'DSH_DESKTOP_CWD', 'DSH_DESKTOP_LOG_DIR',
    'DSH_DESKTOP_USER_HOME', 'DSH_DESKTOP_READY_TIMEOUT_SECS',
    'DSH_DESKTOP_CORE_READY_TIMEOUT_SECS', 'DSH_DESKTOP_PLUGIN_READY_TIMEOUT_SECS',
    'DSH_DESKTOP_WEBVIEW_TEST_DATA_DIR'
)
$previousEnvironment = @{}
foreach ($name in $environmentNames) { $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
$succeeded = $false
try {
    Install-BenchmarkBuild -Context $contexts.legacy -Installer $legacyPath -Payload $false
    Install-BenchmarkBuild -Context $contexts.payload -Installer $payloadPath -Payload $true

    Invoke-BenchmarkRun -Context $contexts.legacy -Label 'legacy-warmup' | Out-Null
    Invoke-BenchmarkRun -Context $contexts.payload -Label 'payload-warmup' | Out-Null

    $warmSamples = [ordered]@{ legacy = @(); payload = @() }
    $orderedPairs = @()
    foreach ($pair in 1..$WarmPairs) {
        $order = if ($pair % 2 -eq 1) { @('legacy', 'payload') } else { @('payload', 'legacy') }
        $pairResult = [ordered]@{ pair = $pair; order = $order; legacyMs = $null; payloadMs = $null }
        foreach ($name in $order) {
            $value = Invoke-BenchmarkRun -Context $contexts[$name] -Label "$name-warm-$pair"
            $warmSamples[$name] += $value
            $pairResult["${name}Ms"] = $value
        }
        $orderedPairs += [pscustomobject]$pairResult
    }

    $coldSamples = [ordered]@{ legacy = @(); payload = @() }
    foreach ($index in 1..$ColdRuns) {
        foreach ($name in @('legacy', 'payload')) {
            $coldHome = Join-Path $contexts[$name].Root "cold-$index.dsh"
            Copy-SeedProfile -Destination $coldHome -Source $seedPath
            $originalHome = $contexts[$name].DshHome
            $contexts[$name].DshHome = $coldHome
            try {
                $coldSamples[$name] += Invoke-BenchmarkRun -Context $contexts[$name] -Label "$name-cold-$index"
            } finally {
                $contexts[$name].DshHome = $originalHome
            }
        }
    }

    $legacyP50 = Get-Percentile -Samples $warmSamples.legacy -Percentile 0.50
    $legacyP95 = Get-Percentile -Samples $warmSamples.legacy -Percentile 0.95
    $payloadP50 = Get-Percentile -Samples $warmSamples.payload -Percentile 0.50
    $payloadP95 = Get-Percentile -Samples $warmSamples.payload -Percentile 0.95
    $allowedRegression = [Math]::Max([Math]::Ceiling($legacyP95 * 0.05), 100)
    $limit = $legacyP95 + $allowedRegression
    $passed = $payloadP95 -le $limit
    $cpu = try { (Get-CimInstance Win32_Processor | Select-Object -First 1 -ExpandProperty Name).Trim() } catch { $env:PROCESSOR_IDENTIFIER }
    $report = [ordered]@{
        schemaVersion = 2
        generatedAtUtc = [DateTime]::UtcNow.ToString('O')
        desktopVersion = $package.version
        sourceCommit = $sourceCommit
        environment = [ordered]@{
            windowsVersion = [Environment]::OSVersion.VersionString
            cpu = $cpu
        }
        installers = [ordered]@{
            legacy = [ordered]@{ version = '0.1.0-preview.7'; path = $legacyPath; sha256 = $actualLegacyHash }
            payload = [ordered]@{ version = $package.version; path = $payloadPath; sha256 = $payloadHash }
        }
        protocol = [ordered]@{
            warmupsPerBuild = 1
            warmPairs = $WarmPairs
            coldRunsPerBuild = $ColdRuns
            alternatingOrder = $true
            measurement = 'external wall clock from Start-Process to host ready log'
            readiness = 'core and plugins ready'
        }
        warmPairs = $orderedPairs
        coldSamplesMs = $coldSamples
        statistics = [ordered]@{
            legacy = [ordered]@{ p50Ms = $legacyP50; p95Ms = $legacyP95; samplesMs = $warmSamples.legacy }
            payload = [ordered]@{ p50Ms = $payloadP50; p95Ms = $payloadP95; samplesMs = $warmSamples.payload }
        }
        gate = [ordered]@{
            formula = 'payloadP95 <= legacyP95 + max(legacyP95 * 5%, 100ms)'
            allowedRegressionMs = $allowedRegression
            payloadP95LimitMs = $limit
            passed = $passed
        }
    }
    New-Item -ItemType Directory -Force -Path (Split-Path $reportPath -Parent) | Out-Null
    $report | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $reportPath -Encoding utf8NoBOM
    if (-not $passed) {
        throw "Payload warm P95 ${payloadP95}ms exceeds legacy ${legacyP95}ms + ${allowedRegression}ms."
    }
    $succeeded = $true
    Write-Host "STARTUP COMPARISON OK: legacy P95=${legacyP95}ms, payload P95=${payloadP95}ms, limit=${limit}ms, report=$reportPath"
}
finally {
    foreach ($context in @($contexts.payload, $contexts.legacy)) {
        foreach ($process in @(Get-OwnedDesktopProcesses $context)) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        }
        try { Uninstall-BenchmarkBuild $context } catch { Write-Warning $_.Exception.Message; $succeeded = $false }
    }
    try {
        Clear-DshInstallerTestUserState -OwnedInstallRoots @($contexts.payload.InstallRoot, $contexts.legacy.InstallRoot)
    } catch {
        Write-Warning $_.Exception.Message
        $succeeded = $false
    }
    foreach ($name in $previousEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process')
    }
    if ($succeeded) {
        foreach ($context in @($contexts.payload, $contexts.legacy)) {
            if (-not (Test-Path -LiteralPath $context.Root -PathType Container)) { continue }
            Remove-DshInstallerTestDirectory -Root $context.Root
        }
    } elseif (-not $succeeded) {
        Write-Warning "Startup benchmark diagnostics retained at $($contexts.legacy.Root) and $($contexts.payload.Root)"
    }
}
