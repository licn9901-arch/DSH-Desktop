param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [int]$WarmRuns = 20,
    [int]$ColdRuns = 3,
    [int]$WarmP95LimitMs = 8000,
    [int]$ColdLimitMs = 20000,
    [string]$DshHome
)

$ErrorActionPreference = 'Stop'
$exePath = [System.IO.Path]::GetFullPath($Exe)
if (-not (Test-Path -LiteralPath $exePath -PathType Leaf)) { throw "Desktop executable not found: $exePath" }
$root = Join-Path ([System.IO.Path]::GetTempPath()) ("dsh-desktop-benchmark-" + [guid]::NewGuid().ToString('N'))
$logDirectory = Join-Path $root 'logs'
$logPath = Join-Path $logDirectory 'dsh-desktop.log'
New-Item -ItemType Directory -Force -Path $logDirectory, (Join-Path $root 'workspace') | Out-Null
$previous = @{
    DSH_HOME = $env:DSH_HOME
    DSH_DESKTOP_LOG_DIR = $env:DSH_DESKTOP_LOG_DIR
    DSH_DESKTOP_CWD = $env:DSH_DESKTOP_CWD
}

function Get-BenchmarkDesktopProcesses {
    @(Get-Process dsh-desktop -ErrorAction SilentlyContinue | Where-Object {
        $_.Path -and [System.IO.Path]::GetFullPath($_.Path) -eq $exePath
    })
}

function Invoke-BenchmarkRun {
    if ((Get-BenchmarkDesktopProcesses).Count -gt 0) {
        throw "Benchmark executable already has a running instance: $exePath"
    }
    $before = if (Test-Path -LiteralPath $logPath) { (Get-Item -LiteralPath $logPath).Length } else { 0 }
    $process = Start-Process -FilePath $exePath -PassThru
    $deadline = (Get-Date).AddSeconds(60)
    $duration = $null
    do {
        Start-Sleep -Milliseconds 100
        if (Test-Path -LiteralPath $logPath) {
            $stream = [System.IO.File]::Open($logPath, 'Open', 'Read', 'ReadWrite')
            try {
                $stream.Position = [Math]::Min($before, $stream.Length)
                $reader = [System.IO.StreamReader]::new($stream)
                $tail = $reader.ReadToEnd()
            } finally { $stream.Dispose() }
            $match = [regex]::Match($tail, 'phase=core_ready duration_ms=(\d+) attempt=')
            if ($match.Success) { $duration = [int]$match.Groups[1].Value; break }
        }
        $process.Refresh()
        if ($process.HasExited) { throw "Desktop exited before CoreReady with code $($process.ExitCode)." }
    } while ((Get-Date) -lt $deadline)
    if ($null -eq $duration) { throw 'Timed out waiting for CoreReady benchmark log.' }
    # 基准只测 CoreReady；生命周期与单实例由独立 smoke 验收，这里按本次记录 PID 做确定性回收。
    Get-BenchmarkDesktopProcesses | Stop-Process -Force -ErrorAction SilentlyContinue
    $hostIds = [regex]::Matches($tail, 'host started: pid=(\d+)') |
        ForEach-Object { [int]$_.Groups[1].Value } | Sort-Object -Unique
    foreach ($hostId in $hostIds) { Stop-Process -Id $hostId -Force -ErrorAction SilentlyContinue }
    $exitDeadline = (Get-Date).AddSeconds(5)
    while ((Get-BenchmarkDesktopProcesses).Count -gt 0 -and (Get-Date) -lt $exitDeadline) {
        Start-Sleep -Milliseconds 100
    }
    if ((Get-BenchmarkDesktopProcesses).Count -gt 0) {
        throw "Benchmark executable remained running after cleanup: $exePath"
    }
    return $duration
}

try {
    $env:DSH_HOME = if ([string]::IsNullOrWhiteSpace($DshHome)) {
        Join-Path $root '.dsh'
    } else {
        [System.IO.Path]::GetFullPath($DshHome)
    }
    $env:DSH_DESKTOP_LOG_DIR = $logDirectory
    $env:DSH_DESKTOP_CWD = Join-Path $root 'workspace'
    Invoke-BenchmarkRun | Out-Null
    $warm = if ($WarmRuns -gt 0) { @(1..$WarmRuns | ForEach-Object { Invoke-BenchmarkRun }) } else { @() }
    if ($warm.Count -eq 0) { throw 'WarmRuns must be greater than zero.' }
    $sorted = @($warm | Sort-Object)
    $p95 = $sorted[[Math]::Ceiling($sorted.Count * 0.95) - 1]
    if ($p95 -gt $WarmP95LimitMs) { throw "Warm CoreReady P95 ${p95}ms exceeds ${WarmP95LimitMs}ms." }

    $cold = foreach ($index in $(if ($ColdRuns -gt 0) { 1..$ColdRuns } else { @() })) {
        $storeRoot = Join-Path $env:DSH_HOME 'profiles\node_modules\.dsh-desktop'
        if (Test-Path -LiteralPath $storeRoot) { Remove-Item -LiteralPath $storeRoot -Recurse -Force }
        Invoke-BenchmarkRun
    }
    $coldFailure = @($cold | Where-Object { $_ -gt $ColdLimitMs })
    if ($coldFailure.Count -gt 0) { throw "Cold CoreReady exceeded ${ColdLimitMs}ms: $($cold -join ', ')" }
    Write-Host "STARTUP BENCHMARK OK: warm P95=${p95}ms ($($warm -join ', ')); cold=$($cold -join ', ')ms."
} finally {
    foreach ($name in $previous.Keys) { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
    if (Test-Path -LiteralPath $logPath) {
        $hostIds = [regex]::Matches((Get-Content -LiteralPath $logPath -Raw), 'host started: pid=(\d+)') |
            ForEach-Object { [int]$_.Groups[1].Value } | Sort-Object -Unique
        foreach ($hostId in $hostIds) { Stop-Process -Id $hostId -Force -ErrorAction SilentlyContinue }
    }
    if (Test-Path -LiteralPath $root) {
        try { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction Stop }
        catch { Write-Warning "Benchmark diagnostics retained at ${root}: $($_.Exception.Message)" }
    }
}
