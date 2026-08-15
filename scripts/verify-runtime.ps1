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

$webPackage = Get-Content -LiteralPath (Join-Path $webRoot 'package.json') -Raw | ConvertFrom-Json
if ($webPackage.version -ne $runtimeLock.dsh.version) {
    throw "Bundled Web frontend version mismatch: $($webPackage.version)"
}

$licenseEntries = @(Get-Content -LiteralPath (Join-Path $hostRoot 'third-party-licenses.json') -Raw | ConvertFrom-Json)
if ($licenseEntries.Count -eq 0) {
    throw 'Third-party license manifest is empty.'
}

Write-Host "Runtime valid: Node $actualNodeVersion, DSH $($dshPackage.version), $($licenseEntries.Count) licensed package entries."
