param([string]$InstallerDirectory = '..\src-tauri\target\release\bundle\nsis')

$ErrorActionPreference = 'Stop'
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
. (Join-Path $PSScriptRoot 'release-source.ps1')
Assert-DshReleaseWorktreeClean -RepoRoot $projectRoot
$sourceCommit = Get-DshReleaseSourceCommit -RepoRoot $projectRoot
$installerRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot $InstallerDirectory))
$deployRoot = [System.IO.Path]::GetFullPath((Join-Path $projectRoot '.deploy-artifacts'))
$runtimeLock = Get-Content -LiteralPath (Join-Path $projectRoot 'runtime.lock.json') -Raw | ConvertFrom-Json
$pluginLock = Get-Content -LiteralPath (Join-Path $projectRoot 'plugins.lock.json') -Raw | ConvertFrom-Json
$package = Get-Content -LiteralPath (Join-Path $projectRoot 'package.json') -Raw | ConvertFrom-Json
$manifestPath = Join-Path $projectRoot 'src-tauri\resources\payload\payload-manifest.json'
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
$reportRoot = Join-Path $projectRoot ".release-work\$($package.version)\reports"
$finalRoot = Join-Path $deployRoot $package.version
$stagingRoot = Join-Path $deployRoot "$($package.version).staging.$PID"
$backupRoot = Join-Path $deployRoot "$($package.version).backup.$PID"

# 读取必需的 JSON 门禁报告，并给缺失文件提供明确错误。
function Read-RequiredReport {
    param([Parameter(Mandatory = $true)][string]$Name)
    $path = Join-Path $reportRoot $Name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Required release report is missing: $path" }
    return [pscustomobject]@{ Path = $path; Value = (Get-Content -LiteralPath $path -Raw | ConvertFrom-Json) }
}

# 验证所有报告都属于当前版本，避免复用旧 preview 证据。
function Assert-ReportVersion {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)]$Report
    )
    if ($Report.desktopVersion -ne $package.version) {
        throw "$Name desktopVersion mismatch: $($Report.desktopVersion) != $($package.version)"
    }
    if ($Report.sourceCommit -ne $sourceCommit) {
        throw "$Name sourceCommit mismatch: $($Report.sourceCommit) != $sourceCommit"
    }
}

# 重建 stage-payload 的完整缓存输入序列，确保 debug symbols 来自当前源码与工具链而非旧 digest 缓存。
function Get-CurrentPayloadCacheInputs {
    $keyInputs = @(
        'runtime.lock.json',
        'plugins.lock.json',
        'runtime-host\package-lock.json',
        'plugin-runtime\package-lock.json',
        'package-lock.json',
        'src-tauri\Cargo.toml',
        'src-tauri\Cargo.lock',
        'src-tauri\src\payload.rs',
        'src-tauri\examples\payload-tool.rs',
        'rust-toolchain.toml',
        'scripts\stage-runtime.ps1',
        'scripts\patch-directory-picker.ps1',
        'scripts\stage-plugins.ps1',
        'scripts\optimize-plugin-previews.mjs',
        'scripts\prune-plugin-client-dependencies.mjs',
        'scripts\stage-payload.ps1'
    )
    $lines = @(
        'platform=win32-x64',
        "node=$(& node.exe --version)",
        "npm=$(& npm.cmd --version)",
        "esbuild=$(& node.exe -p "require('./node_modules/esbuild/package.json').version")",
        "rust=$(& rustc.exe --version)"
    )
    foreach ($relative in $keyInputs) {
        $path = Join-Path $projectRoot $relative
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Payload cache input is missing: $path" }
        $lines += "$relative=$((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant())"
    }
    return $lines
}

$installerPattern = "*_$($package.version)_x64-setup.exe"
$installers = @(Get-ChildItem -LiteralPath $installerRoot -Filter $installerPattern -File)
if ($installers.Count -ne 1) { throw "Expected exactly one $installerPattern installer, found $($installers.Count)." }
$installer = $installers[0]
$installerHash = (Get-FileHash -LiteralPath $installer.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
if ($installer.Length -gt 100MB) { throw "Installer exceeds 100 MiB: $($installer.Length) bytes." }

if ($manifest.desktopVersion -ne $package.version -or
    $manifest.nodeVersion -ne $runtimeLock.node.version -or
    $manifest.pnpmVersion -ne $runtimeLock.pnpm.version) {
    throw 'Payload manifest does not match package/runtime lock versions.'
}
if (@(Get-ChildItem -LiteralPath (Split-Path $manifestPath -Parent) -File).Count -ne 4) {
    throw 'Payload resource directory must contain exactly manifest plus three ZIP files.'
}

$audit = Read-RequiredReport 'npm-audit.json'
$compatibility = Read-RequiredReport 'pnpm-compatibility.json'
$reproducibility = Read-RequiredReport 'payload-reproducibility.json'
$startup = Read-RequiredReport 'startup-comparison.json'
$upgrade = Read-RequiredReport 'upgrade-matrix.json'
$build = Read-RequiredReport 'payload-build-report.json'
$gate = Read-RequiredReport 'release-gate.json'
$gateMarkdown = Join-Path $reportRoot 'release-gate.md'
if (-not (Test-Path -LiteralPath $gateMarkdown -PathType Leaf)) { throw "Required release report is missing: $gateMarkdown" }
foreach ($named in @(
    @{ Name = 'npm-audit'; Report = $audit.Value },
    @{ Name = 'pnpm-compatibility'; Report = $compatibility.Value },
    @{ Name = 'payload-reproducibility'; Report = $reproducibility.Value },
    @{ Name = 'startup-comparison'; Report = $startup.Value },
    @{ Name = 'upgrade-matrix'; Report = $upgrade.Value },
    @{ Name = 'payload-build-report'; Report = $build.Value },
    @{ Name = 'release-gate'; Report = $gate.Value }
)) { Assert-ReportVersion $named.Name $named.Report }

$auditTotals = @($audit.Value.projects | ForEach-Object { $_.metadata.vulnerabilities })
if (@($auditTotals | Where-Object { $_.total -ne 0 -or $_.high -ne 0 -or $_.critical -ne 0 }).Count -gt 0) {
    throw 'npm audit report contains residual advisories.'
}
if ($compatibility.Value.fixedPnpmVersion -ne $runtimeLock.pnpm.version -or $compatibility.Value.cases.Count -ne 3) {
    throw 'pnpm compatibility report does not cover the approved fixed version and three historical profiles.'
}
if (-not $reproducibility.Value.passed) { throw 'Payload reproducibility gate did not pass.' }
if (-not $startup.Value.gate.passed) { throw 'Startup P95 comparison gate did not pass.' }
if (-not $upgrade.Value.passed) { throw 'Installer upgrade matrix did not pass.' }
if (-not $gate.Value.passed) { throw 'Unified release gate did not pass.' }
if ($build.Value.schemaVersion -ne 3 -or
    $build.Value.nodeVersion -ne $runtimeLock.node.version -or
    $build.Value.pnpmVersion -ne $runtimeLock.pnpm.version -or
    $build.Value.marketVersion -ne $runtimeLock.market.version -or
    $build.Value.payloadDigest -ne $manifest.payloadDigest -or
    $build.Value.payloadResourceFiles -ne 4 -or
    $build.Value.installer.sha256 -ne $installerHash -or
    $build.Value.installer.bytes -ne $installer.Length) {
    throw 'Payload build report does not match current lock, manifest or installer.'
}

$currentCacheInputs = @(Get-CurrentPayloadCacheInputs)
$currentCacheInputText = $currentCacheInputs -join "`n"
$matchingCache = @(Get-ChildItem -LiteralPath (Join-Path $projectRoot '.runtime-cache\payload') -Directory -ErrorAction SilentlyContinue | Where-Object {
    $cachedManifest = Join-Path $_.FullName 'resources\payload-manifest.json'
    $cachedReport = Join-Path $_.FullName 'build-report.json'
    if (-not (Test-Path -LiteralPath $cachedManifest -PathType Leaf) -or
        -not (Test-Path -LiteralPath $cachedReport -PathType Leaf)) { return $false }
    $cachedManifestValue = Get-Content -LiteralPath $cachedManifest -Raw | ConvertFrom-Json
    $cachedReportValue = Get-Content -LiteralPath $cachedReport -Raw | ConvertFrom-Json
    $cachedManifestValue.payloadDigest -eq $manifest.payloadDigest -and
        $cachedReportValue.payloadDigest -eq $manifest.payloadDigest -and
        $cachedReportValue.cacheKey -eq $_.Name -and
        ((@($cachedReportValue.inputs) | ForEach-Object { [string]$_ }) -join "`n") -ceq $currentCacheInputText
})
if ($matchingCache.Count -ne 1) {
    throw "Expected exactly one payload cache matching the current inputs and digest $($manifest.payloadDigest), found $($matchingCache.Count)."
}
$debugArchive = Join-Path $deployRoot "runtime-debug-symbols\runtime-debug-symbols-$($matchingCache[0].Name).zip"
if (-not (Test-Path -LiteralPath $debugArchive -PathType Leaf)) { throw "Debug symbols archive is missing: $debugArchive" }

$licenseSources = [ordered]@{
    'third-party-licenses.json' = Join-Path $projectRoot 'src-tauri\resources\host\third-party-licenses.json'
    'plugin-third-party-licenses.json' = Join-Path $projectRoot 'src-tauri\resources\plugins\third-party-licenses.json'
    'plugins.lock.json' = Join-Path $projectRoot 'plugins.lock.json'
    'THIRD_PARTY_NOTICES.md' = Join-Path $projectRoot 'THIRD_PARTY_NOTICES.md'
    'RELEASE_NOTES.md' = Join-Path $projectRoot 'RELEASE_NOTES.md'
}
foreach ($source in $licenseSources.Values) {
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Release license input is missing: $source" }
}

New-Item -ItemType Directory -Force -Path $deployRoot | Out-Null
if (Test-Path -LiteralPath $stagingRoot) { Remove-Item -LiteralPath $stagingRoot -Recurse -Force }
if (Test-Path -LiteralPath $backupRoot) { throw "Release backup path already exists: $backupRoot" }
New-Item -ItemType Directory -Path $stagingRoot | Out-Null

$publishedInstallerName = $installer.Name.Replace(' ', '.')
$publishedInstaller = Join-Path $stagingRoot $publishedInstallerName
Copy-Item -LiteralPath $installer.FullName -Destination $publishedInstaller
"$installerHash  $publishedInstallerName" | Set-Content -LiteralPath "$publishedInstaller.sha256" -Encoding ascii
Copy-Item -LiteralPath $manifestPath -Destination (Join-Path $stagingRoot 'payload-manifest.json')
Copy-Item -LiteralPath $debugArchive -Destination (Join-Path $stagingRoot 'runtime-debug-symbols.zip')
foreach ($entry in $licenseSources.GetEnumerator()) {
    Copy-Item -LiteralPath $entry.Value -Destination (Join-Path $stagingRoot $entry.Key)
}
foreach ($report in @($audit, $compatibility, $reproducibility, $startup, $upgrade, $build, $gate)) {
    Copy-Item -LiteralPath $report.Path -Destination (Join-Path $stagingRoot (Split-Path $report.Path -Leaf))
}
Copy-Item -LiteralPath $gateMarkdown -Destination $stagingRoot

$sizeMiB = [Math]::Round($installer.Length / 1MB, 2)
$pluginList = ($pluginLock.plugins | ForEach-Object { "$($_.package)@$($_.version)" }) -join ', '
@"
# Build Summary

- Version: $($package.version)
- Source commit: $sourceCommit
- Target: Windows x64 NSIS payload preview
- Node.js: $($runtimeLock.node.version)
- @deepseek-ai/dsh: $($runtimeLock.dsh.version)
- dshmarket: $($runtimeLock.market.version)
- pnpm: $($runtimeLock.pnpm.version)
- Payload digest: $($manifest.payloadDigest)
- Payload resources: 4
- Managed plugins: $pluginList
- Installer: $publishedInstallerName
- Installer size: $($installer.Length) bytes ($sizeMiB MiB)
- SHA-256: $installerHash
- Authenticode: unsigned preview
- Generated at: $([DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ'))
"@ | Set-Content -LiteralPath (Join-Path $stagingRoot 'build-summary.md') -Encoding utf8NoBOM

$stagedFiles = @(Get-ChildItem -LiteralPath $stagingRoot -File)
foreach ($required in @(
    $publishedInstallerName, "$publishedInstallerName.sha256", 'payload-manifest.json',
    'payload-build-report.json', 'npm-audit.json', 'pnpm-compatibility.json',
    'payload-reproducibility.json', 'startup-comparison.json', 'upgrade-matrix.json',
    'release-gate.json', 'release-gate.md',
    'runtime-debug-symbols.zip', 'third-party-licenses.json', 'plugin-third-party-licenses.json',
    'plugins.lock.json', 'THIRD_PARTY_NOTICES.md', 'RELEASE_NOTES.md', 'build-summary.md'
)) {
    if ($required -notin $stagedFiles.Name) { throw "Staged release artifact is missing: $required" }
}

$movedOld = $false
try {
    if (Test-Path -LiteralPath $finalRoot) {
        Move-Item -LiteralPath $finalRoot -Destination $backupRoot
        $movedOld = $true
    }
    Move-Item -LiteralPath $stagingRoot -Destination $finalRoot
    if ($movedOld) { Remove-Item -LiteralPath $backupRoot -Recurse -Force }
}
catch {
    if (-not (Test-Path -LiteralPath $finalRoot) -and $movedOld -and (Test-Path -LiteralPath $backupRoot)) {
        Move-Item -LiteralPath $backupRoot -Destination $finalRoot
    }
    throw
}

Write-Host "Release artifacts atomically published to $finalRoot"
