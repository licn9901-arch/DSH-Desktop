param(
    [string]$ResourceRoot = '..\src-tauri\resources',
    [string]$ArchivePath
)

$ErrorActionPreference = 'Stop'

$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ([System.IO.Path]::IsPathRooted($ResourceRoot)) {
    $resourceRootPath = [System.IO.Path]::GetFullPath($ResourceRoot)
}
else {
    $resourceRootPath = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot $ResourceRoot))
}
$runtimeLock = Get-Content -LiteralPath (Join-Path $projectRoot 'runtime.lock.json') -Raw | ConvertFrom-Json
$nodeRoot = Join-Path $resourceRootPath 'node'
$hostRoot = Join-Path $resourceRootPath 'host'
$nodePath = Join-Path $nodeRoot 'node.exe'
$cliPath = Join-Path $hostRoot $runtimeLock.dsh.cliEntry
$dshRoot = Join-Path $hostRoot 'node_modules\@deepseek-ai\dsh'
$marketRoot = Join-Path $hostRoot 'node_modules\dshmarket'
$pnpmRoot = Join-Path $hostRoot 'node_modules\pnpm'
$policyPath = Join-Path $resourceRootPath 'policy\dsh-market.patch.yml'
$webCandidates = @(
    (Join-Path $hostRoot 'node_modules\@deepseek-ai\dsh-web-frontend'),
    (Join-Path $dshRoot 'node_modules\@deepseek-ai\dsh-web-frontend')
)
$webRoot = $webCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Container } | Select-Object -First 1
if (-not $webRoot) {
    throw "Bundled Web frontend package is missing. Checked: $($webCandidates -join '; ')"
}

$requiredFiles = @(
    $nodePath,
    (Join-Path $nodeRoot 'LICENSE'),
    (Join-Path $nodeRoot 'runtime.lock.json'),
    $cliPath,
    (Join-Path $dshRoot 'package.json'),
    (Join-Path $dshRoot 'LICENSE'),
    (Join-Path $marketRoot 'package.json'),
    (Join-Path $marketRoot 'LICENSE'),
    (Join-Path $marketRoot 'cordis.patch.yml'),
    (Join-Path $pnpmRoot 'package.json'),
    (Join-Path $pnpmRoot 'LICENSE'),
    (Join-Path $pnpmRoot 'bin\pnpm.mjs'),
    (Join-Path $hostRoot 'node_modules\.bin\pnpm.cmd'),
    (Join-Path $hostRoot 'toolchains\pnpm-9\pnpm.cmd'),
    (Join-Path $hostRoot 'toolchains\pnpm-10\pnpm.cmd'),
    (Join-Path $hostRoot 'toolchains\pnpm-11\pnpm.cmd'),
    $policyPath,
    (Join-Path $webRoot 'dist\index.html'),
    (Join-Path $webRoot 'LICENSE'),
    (Join-Path $hostRoot 'package-lock.json'),
    (Join-Path $hostRoot 'THIRD_PARTY_NOTICES.md'),
    (Join-Path $hostRoot 'third-party-licenses.json')
)
foreach ($requiredFile in $requiredFiles) {
    if (-not (Test-Path -LiteralPath $requiredFile -PathType Leaf)) {
        throw "Bundled runtime file is missing: $requiredFile"
    }
}

# 两个资源分区都必须携带当前根锁文件，避免增量 staging 留下旧版本声明。
$rootLockHash = (Get-FileHash -LiteralPath (Join-Path $projectRoot 'runtime.lock.json') -Algorithm SHA256).Hash
foreach ($stagedLock in @(
    (Join-Path $nodeRoot 'runtime.lock.json'),
    (Join-Path $hostRoot 'runtime.lock.json')
)) {
    $stagedLockHash = (Get-FileHash -LiteralPath $stagedLock -Algorithm SHA256).Hash
    if ($stagedLockHash -ne $rootLockHash) {
        throw "Staged runtime lock is stale: $stagedLock"
    }
}

if ($ArchivePath) {
    $resolvedArchive = [System.IO.Path]::GetFullPath($ArchivePath)
    $actualHash = (Get-FileHash -LiteralPath $resolvedArchive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $runtimeLock.node.sha256.ToLowerInvariant()) {
        throw "Cached Node archive SHA-256 mismatch: $actualHash"
    }
}

$actualNodeVersion = (& $nodePath --version).TrimStart('v').Trim()
if ($LASTEXITCODE -ne 0 -or $actualNodeVersion -ne $runtimeLock.node.version) {
    throw "Bundled Node version mismatch. Expected $($runtimeLock.node.version), got $actualNodeVersion."
}

$dshPackage = Get-Content -LiteralPath (Join-Path $dshRoot 'package.json') -Raw | ConvertFrom-Json
if ($dshPackage.name -ne $runtimeLock.dsh.package -or $dshPackage.version -ne $runtimeLock.dsh.version) {
    throw "Bundled DSH package mismatch: $($dshPackage.name) $($dshPackage.version)"
}

$hostLock = Get-Content -LiteralPath (Join-Path $hostRoot 'package-lock.json') -Raw | ConvertFrom-Json -AsHashtable
$lockedDsh = $hostLock['packages']['node_modules/@deepseek-ai/dsh']
if ($lockedDsh['version'] -ne $runtimeLock.dsh.version -or $lockedDsh['integrity'] -ne $runtimeLock.dsh.integrity) {
    throw 'Bundled DSH lock entry does not match runtime.lock.json.'
}

$marketPackage = Get-Content -LiteralPath (Join-Path $marketRoot 'package.json') -Raw | ConvertFrom-Json
$lockedMarket = $hostLock['packages']['node_modules/dshmarket']
if ($marketPackage.name -ne $runtimeLock.market.package -or
    $marketPackage.version -ne $runtimeLock.market.version -or
    $lockedMarket['version'] -ne $runtimeLock.market.version -or
    $lockedMarket['integrity'] -ne $runtimeLock.market.integrity) {
    throw 'Bundled DSH Market does not match runtime.lock.json.'
}

$sourceClient = Get-Content -LiteralPath (Join-Path $marketRoot 'client\client.js') -Raw
$sourceRegistration = 'window.__ModuleLoader__.load({ id: "dshmarket", factory:'
if (-not $sourceClient.StartsWith($sourceRegistration, [System.StringComparison]::Ordinal)) {
    throw 'Bundled Market client must keep the upstream dshmarket registration ID.'
}

$pnpmPackage = Get-Content -LiteralPath (Join-Path $pnpmRoot 'package.json') -Raw | ConvertFrom-Json
$lockedPnpm = $hostLock['packages']['node_modules/pnpm']
if ($pnpmPackage.name -ne $runtimeLock.pnpm.package -or
    $pnpmPackage.version -ne $runtimeLock.pnpm.version -or
    $lockedPnpm['version'] -ne $runtimeLock.pnpm.version -or
    $lockedPnpm['integrity'] -ne $runtimeLock.pnpm.integrity) {
    throw 'Bundled pnpm does not match runtime.lock.json.'
}
if ($pnpmPackage.engines.node -ne $runtimeLock.pnpm.nodeRange) {
    throw "Bundled pnpm Node compatibility mismatch: $($pnpmPackage.engines.node)"
}
foreach ($toolchain in $runtimeLock.pnpmToolchains) {
    $entry = $hostLock['packages']['node_modules/' + $toolchain.package]
    $packagePath = Join-Path $hostRoot ('node_modules\' + $toolchain.package + '\package.json')
    $package = Get-Content -LiteralPath $packagePath -Raw | ConvertFrom-Json
    if ($package.version -ne $toolchain.version -or
        $entry['version'] -ne $toolchain.version -or
        $entry['integrity'] -ne $toolchain.integrity) {
        throw "Bundled $($toolchain.package) does not match runtime.lock.json."
    }
}

$policy = Get-Content -LiteralPath $policyPath -Raw
if ($policy -notmatch '(?m)^- id:\s*dsh-market\s*$' -or
    $policy -notmatch '(?m)^\s*profile:\s*web\s*$' -or
    $policy -notmatch '(?m)^\s*allowRestart:\s*false\s*$') {
    throw 'Desktop DSH Market policy must configure the upstream dsh-market entry with profile=web and allowRestart=false.'
}

$webPackage = Get-Content -LiteralPath (Join-Path $webRoot 'package.json') -Raw | ConvertFrom-Json
if ($webPackage.version -ne $runtimeLock.dsh.version) {
    throw "Bundled Web frontend version mismatch: $($webPackage.version)"
}

$licenseEntries = @(Get-Content -LiteralPath (Join-Path $hostRoot 'third-party-licenses.json') -Raw | ConvertFrom-Json)
if ($licenseEntries.Count -eq 0) {
    throw 'Third-party license manifest is empty.'
}

Write-Host "Runtime valid: Node $actualNodeVersion, DSH $($dshPackage.version), Market $($marketPackage.version), pnpm $($pnpmPackage.version), $($licenseEntries.Count) licensed package entries."
