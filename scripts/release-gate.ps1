param(
    [Parameter(Mandatory = $true)][string]$LegacyInstaller,
    [Parameter(Mandatory = $true)][string]$PayloadInstaller,
    [string]$PreviousPayloadInstaller,
    [string]$PreviousPayloadAssetDigest
)

$ErrorActionPreference = 'Stop'
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
. (Join-Path $PSScriptRoot 'release-installer-isolation.ps1')
. (Join-Path $PSScriptRoot 'release-source.ps1')
. (Join-Path $PSScriptRoot 'release-gate-helpers.ps1')
$sourceCommit = Get-DshReleaseSourceCommit -RepoRoot $repoRoot
$package = Get-Content -LiteralPath (Join-Path $repoRoot 'package.json') -Raw | ConvertFrom-Json
$runtimeLock = Get-Content -LiteralPath (Join-Path $repoRoot 'runtime.lock.json') -Raw | ConvertFrom-Json
$preview = Get-DshPreviewNumber -Version $package.version
$reportRoot = Join-Path $repoRoot ".release-work\$($package.version)\reports"
$jsonPath = Join-Path $reportRoot 'release-gate.json'
$markdownPath = Join-Path $reportRoot 'release-gate.md'
New-Item -ItemType Directory -Force -Path $reportRoot | Out-Null

# 将用户输入路径解析为稳定绝对路径，报告和子门禁共享同一安装器。
function Resolve-GateFile {
    param([Parameter(Mandatory = $true)][string]$Path)
    $resolved = if ([System.IO.Path]::IsPathRooted($Path)) {
        [System.IO.Path]::GetFullPath($Path)
    } else {
        [System.IO.Path]::GetFullPath((Join-Path $repoRoot $Path))
    }
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) { throw "Release gate input is missing: $resolved" }
    return $resolved
}

# 验证前序公开 payload preview 已完成同一套门禁。
function Assert-PreviousPreviewPassed {
    param([Parameter(Mandatory = $true)][int]$Number)
    $path = Join-Path $repoRoot ".deploy-artifacts\0.1.0-preview.$Number\release-gate.json"
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Previous preview gate report is missing: $path" }
    $report = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
    if (-not $report.passed) { throw "Previous preview.$Number release gate did not pass." }
    if ($report.desktopVersion -ne "0.1.0-preview.$Number") {
        throw "Previous preview.$Number release report version mismatch: $($report.desktopVersion)"
    }
    return $report
}

# 要求当前门禁消费的报告来自同一版本和同一 Git 提交。
function Assert-CurrentReleaseReport {
    param([Parameter(Mandatory = $true)][string]$Name)
    $path = Join-Path $reportRoot $Name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Current release report is missing: $path" }
    $report = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
    if ($report.desktopVersion -ne $package.version -or $report.sourceCommit -ne $sourceCommit) {
        throw "Current release report identity mismatch: $Name"
    }
}

$legacyPath = Resolve-GateFile $LegacyInstaller
$payloadPath = Resolve-GateFile $PayloadInstaller
$gitSafeDirectory = $repoRoot.Replace('\', '/')
$previousPayloadPath = if ([string]::IsNullOrWhiteSpace($PreviousPayloadInstaller)) { $null } else { Resolve-GateFile $PreviousPayloadInstaller }
$expectedBaselineHash = 'e331e628b07bf574e823610324130c258d77ed1e57113b59426feed1a3a9d3d9'
$legacyHash = (Get-FileHash -LiteralPath $legacyPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($legacyHash -ne $expectedBaselineHash) { throw "preview.7 baseline hash mismatch: $legacyHash" }
if ($runtimeLock.pnpm.version -ne '10.34.5' -or
    $runtimeLock.pnpm.integrity -ne 'sha512-pO4F8vc2WCVb1qiYWcBlpFwopX2u+uLIk6Fo7itzFow3uR6D5X6mdlStA/AwMXRkMOi84442LgQmBfuKvIAZLg==') {
    throw 'runtime.lock.json does not contain the approved pnpm 10.34.5 package and integrity.'
}
if ($preview -le 9 -and $package.scripts.build -ne 'npm run build:legacy') {
    throw "preview.$preview must keep npm run build on legacy."
}
if ($preview -ge 10 -and $package.scripts.build -ne 'npm run build:payload') {
    throw "preview.$preview must use npm run build:payload."
}
if ($preview -ge 9 -and $null -eq $previousPayloadPath) {
    throw "preview.$preview requires -PreviousPayloadInstaller."
}
$previousReports = @{}
$previousEvidence = $null
if ($preview -ge 9) {
    foreach ($number in 8..([Math]::Min(11, $preview - 1))) {
        $previousReports[$number] = Assert-PreviousPreviewPassed $number
    }
    $previousVersion = "0.1.0-preview.$($preview - 1)"
    $previousGatePath = Join-Path $repoRoot ".deploy-artifacts\$previousVersion\release-gate.json"
    $previousEvidence = Get-DshPreviousInstallerEvidence `
        -InstallerPath $previousPayloadPath `
        -ExpectedVersion $previousVersion `
        -GateReportPath $previousGatePath `
        -AssetDigest $PreviousPayloadAssetDigest
}

$phases = @()

# 执行一个门禁阶段，记录耗时与退出状态，并在失败时立即停止后续发布验证。
function Invoke-GatePhase {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Command,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    Write-Host "RELEASE GATE START: $Name"
    & $Command @Arguments
    $exitCode = $LASTEXITCODE
    $watch.Stop()
    $script:phases += [pscustomobject]@{
        name = $Name
        command = "$Command $($Arguments -join ' ')"
        exitCode = $exitCode
        durationMs = $watch.ElapsedMilliseconds
        passed = $exitCode -eq 0
    }
    if ($exitCode -ne 0) { throw "Release gate phase failed: $Name (exit code $exitCode)" }
    Write-Host "RELEASE GATE END: $Name elapsedMs=$($watch.ElapsedMilliseconds)"
}

$passed = $false
$failure = $null
$totalWatch = [System.Diagnostics.Stopwatch]::StartNew()
Push-Location $repoRoot
try {
    try {
        Assert-DshReleaseWorktreeClean -RepoRoot $repoRoot
        $phases += [pscustomobject]@{ name = 'gitWorktreeClean'; command = 'git status --porcelain=v1 --untracked-files=all'; exitCode = 0; durationMs = 0; passed = $true }
    }
    catch {
        $phases += [pscustomobject]@{ name = 'gitWorktreeClean'; command = 'git status --porcelain=v1 --untracked-files=all'; exitCode = 1; durationMs = 0; passed = $false }
        throw
    }
    $isolationConflicts = @(Get-DshInstallerUserStateConflicts)
    if ($isolationConflicts.Count -gt 0) {
        $phases += [pscustomobject]@{ name = 'installerIsolationPreflight'; command = 'process, HKCU and shortcut inspection'; exitCode = 1; durationMs = 0; passed = $false }
        throw "Release installer gates require a clean disposable Windows user: $($isolationConflicts -join '; ')."
    }
    $phases += [pscustomobject]@{ name = 'installerIsolationPreflight'; command = 'process, HKCU and shortcut inspection'; exitCode = 0; durationMs = 0; passed = $true }
    Invoke-GatePhase 'gitDiffCheck' 'git.exe' @('-C', $repoRoot, '-c', "safe.directory=$gitSafeDirectory", 'diff', '--check')
    Invoke-GatePhase 'lint' 'npm.cmd' @('run', 'lint')
    Invoke-GatePhase 'tests' 'npm.cmd' @('test')
    Invoke-GatePhase 'coverage' 'npm.cmd' @('run', 'coverage')
    Invoke-GatePhase 'runtimeVerify' 'npm.cmd' @('run', 'verify:runtime')
    Invoke-GatePhase 'pluginVerify' 'npm.cmd' @('run', 'verify:plugins')
    Invoke-GatePhase 'payloadVerify' 'npm.cmd' @('run', 'verify:payload')
    Invoke-GatePhase 'npmAudit' 'npm.cmd' @('run', 'audit:release')
    Invoke-GatePhase 'pnpmCompatibility' 'npm.cmd' @('run', 'test:pnpm-compat')
    Invoke-GatePhase 'payloadReproducibility' 'pwsh.exe' @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $PSScriptRoot 'verify-payload-reproducibility.ps1')
    )
    $upgradeArguments = @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $PSScriptRoot 'upgrade-matrix.ps1'),
        '-LegacyInstaller', $legacyPath, '-PayloadInstaller', $payloadPath, '-TimeoutSeconds', '180'
    )
    if ($null -ne $previousPayloadPath) { $upgradeArguments += @('-PreviousPayloadInstaller', $previousPayloadPath) }
    Invoke-GatePhase 'upgradeMatrix' 'pwsh.exe' $upgradeArguments
    Invoke-GatePhase 'startupComparison' 'pwsh.exe' @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $PSScriptRoot 'benchmark-compare.ps1'),
        '-LegacyInstaller', $legacyPath, '-PayloadInstaller', $payloadPath,
        '-WarmPairs', '20', '-ColdRuns', '3', '-TimeoutSeconds', '180'
    )
    foreach ($reportName in @(
        'payload-build-report.json', 'npm-audit.json', 'pnpm-compatibility.json',
        'payload-reproducibility.json', 'upgrade-matrix.json', 'startup-comparison.json'
    )) { Assert-CurrentReleaseReport $reportName }
    $phases += [pscustomobject]@{ name = 'reportIdentity'; command = 'version and sourceCommit comparison'; exitCode = 0; durationMs = 0; passed = $true }
    $passed = $true
}
catch {
    $failure = $_.Exception.Message
}
finally {
    Pop-Location
    $totalWatch.Stop()
    $report = [ordered]@{
        schemaVersion = 2
        generatedAtUtc = [DateTime]::UtcNow.ToString('O')
        desktopVersion = $package.version
        sourceCommit = $sourceCommit
        previewNumber = $preview
        previousPayloadEvidence = $previousEvidence
        installers = [ordered]@{
            legacy = [ordered]@{ path = $legacyPath; sha256 = $legacyHash }
            payload = [ordered]@{
                path = $payloadPath
                sha256 = (Get-FileHash -LiteralPath $payloadPath -Algorithm SHA256).Hash.ToLowerInvariant()
            }
            previousPayload = if ($null -eq $previousPayloadPath) { $null } else { [ordered]@{
                path = $previousPayloadPath
                sha256 = (Get-FileHash -LiteralPath $previousPayloadPath -Algorithm SHA256).Hash.ToLowerInvariant()
            } }
        }
        totalMs = $totalWatch.ElapsedMilliseconds
        phases = $phases
        passed = $passed
        failure = $failure
    }
    $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $jsonPath -Encoding utf8NoBOM
    $phaseLines = @($phases | ForEach-Object {
        $status = if ($_.passed) { 'PASS' } else { 'FAIL' }
        "| $($_.name) | $status | $($_.durationMs) |"
    }) -join "`n"
    $status = if ($passed) { 'PASS' } else { 'FAIL' }
    @"
# Release Gate $($package.version)

- Status: $status
- Source commit: $sourceCommit
- pnpm: $($runtimeLock.pnpm.version)
- Legacy baseline SHA-256: $legacyHash
- Payload installer SHA-256: $((Get-FileHash -LiteralPath $payloadPath -Algorithm SHA256).Hash.ToLowerInvariant())
- Total: $($totalWatch.ElapsedMilliseconds) ms
- Failure: $(if ($null -eq $failure) { 'none' } else { $failure })

| Phase | Status | Duration (ms) |
| --- | --- | ---: |
$phaseLines
"@ | Set-Content -LiteralPath $markdownPath -Encoding utf8NoBOM
}

if (-not $passed) { throw "Release gate failed: $failure; report=$jsonPath" }
Write-Host "RELEASE GATE OK: $($package.version), report=$jsonPath"
