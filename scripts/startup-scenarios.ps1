param(
    [string]$Exe = '..\src-tauri\target\debug\dsh-desktop.exe',
    [int]$TimeoutSeconds = 60
)

$ErrorActionPreference = 'Stop'
$exePath = if ([System.IO.Path]::IsPathRooted($Exe)) {
    [System.IO.Path]::GetFullPath($Exe)
} else {
    [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot $Exe))
}
$fakeHost = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'fixtures\fake-host.js'))
$node = (Get-Command node.exe -ErrorAction Stop).Source

function Invoke-StartupScenario {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][scriptblock]$AssertLog
    )

    $root = Join-Path ([System.IO.Path]::GetTempPath()) ("dsh-desktop-startup-$Name-" + [guid]::NewGuid().ToString('N'))
    $logDirectory = Join-Path $root 'logs'
    $logPath = Join-Path $logDirectory 'dsh-desktop.log'
    New-Item -ItemType Directory -Force -Path $logDirectory, (Join-Path $root 'workspace') | Out-Null
    $process = $null
    try {
        $env:DSH_HOME = Join-Path $root '.dsh'
        $env:DSH_DESKTOP_LOG_DIR = $logDirectory
        $env:DSH_DESKTOP_NODE_EXECUTABLE = $node
        $env:DSH_DESKTOP_CLI_ENTRY = $fakeHost
        $env:DSH_DESKTOP_CWD = Join-Path $root 'workspace'
        $env:DSH_DESKTOP_USER_HOME = $root
        $env:DSH_DESKTOP_CORE_READY_TIMEOUT_SECS = '3'
        $env:DSH_DESKTOP_PLUGIN_READY_TIMEOUT_SECS = '2'
        $env:DSH_DESKTOP_FAKE_HOST_SCENARIO = $Name
        $env:DSH_DESKTOP_FAKE_PLUGIN_DELAY_MS = if ($Name -eq 'core-first') { '700' } else { '150' }

        $process = Start-Process -FilePath $exePath -PassThru
        $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
        do {
            Start-Sleep -Milliseconds 100
            $content = if (Test-Path -LiteralPath $logPath) { Get-Content -LiteralPath $logPath -Raw } else { '' }
            if (& $AssertLog $content) { return }
            $process.Refresh()
        } while ((Get-Date) -lt $deadline -and -not $process.HasExited)
        throw "Startup scenario '$Name' did not reach its expected state.`n$content"
    }
    finally {
        if ($process -and -not $process.HasExited) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        }
        if (Test-Path -LiteralPath $logPath) {
            $hostIds = [regex]::Matches((Get-Content -LiteralPath $logPath -Raw), 'host started: pid=(\d+)') |
                ForEach-Object { [int]$_.Groups[1].Value } | Sort-Object -Unique
            foreach ($hostId in $hostIds) {
                Stop-Process -Id $hostId -Force -ErrorAction SilentlyContinue
            }
        }
        if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
    }
}

$previous = @{}
foreach ($name in @(
    'DSH_HOME', 'DSH_DESKTOP_LOG_DIR', 'DSH_DESKTOP_NODE_EXECUTABLE',
    'DSH_DESKTOP_CLI_ENTRY', 'DSH_DESKTOP_CWD', 'DSH_DESKTOP_USER_HOME',
    'DSH_DESKTOP_CORE_READY_TIMEOUT_SECS', 'DSH_DESKTOP_PLUGIN_READY_TIMEOUT_SECS',
    'DSH_DESKTOP_FAKE_HOST_SCENARIO', 'DSH_DESKTOP_FAKE_PLUGIN_DELAY_MS'
)) { $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }

try {
    Invoke-StartupScenario -Name 'core-first' -AssertLog {
        param($log)
        $core = $log.IndexOf('phase=core_ready')
        $plugins = $log.IndexOf('phase=plugins_ready')
        $core -ge 0 -and $plugins -gt $core
    }
    Invoke-StartupScenario -Name 'core-crash' -AssertLog {
        param($log)
        ([regex]::Matches($log, 'host started: pid=').Count -eq 2) -and
            $log.Contains('phase=rollback') -and $log.Contains('attempt=core')
    }
    Invoke-StartupScenario -Name 'plugins-never' -AssertLog {
        param($log)
        ([regex]::Matches($log, 'host started: pid=').Count -eq 2) -and
            $log.Contains('phase=rollback') -and
            ([regex]::Matches($log, 'phase=core_ready').Count -eq 2) -and
            $log.Contains('phase=plugins_degraded')
    }
} finally {
    foreach ($name in $previous.Keys) {
        [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process')
    }
}

Write-Host 'STARTUP SCENARIOS OK: core-first, core-crash and plugins-never recovery verified.'
