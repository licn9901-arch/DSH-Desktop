param(
    [switch]$Offline
)

$ErrorActionPreference = 'Stop'

$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$lockPath = Join-Path $projectRoot 'runtime.lock.json'
$runtimeHostRoot = Join-Path $projectRoot 'runtime-host'
$cacheRoot = Join-Path $projectRoot '.runtime-cache'
$resourceRoot = Join-Path $projectRoot 'src-tauri\resources'
$nodeResourceRoot = Join-Path $resourceRoot 'node'
$hostResourceRoot = Join-Path $resourceRoot 'host'
$npmCache = Join-Path $cacheRoot 'npm-cache'

# 删除或覆盖前验证目标仍位于仓库根目录，防止变量异常扩大影响范围。
function Assert-ProjectPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $resolved = [System.IO.Path]::GetFullPath($Path)
    $prefix = $projectRoot.TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to modify a path outside the project: $resolved"
    }
    return $resolved
}

if (-not (Test-Path -LiteralPath $lockPath -PathType Leaf)) {
    throw "Runtime lock file is missing: $lockPath"
}
$runtimeLock = Get-Content -LiteralPath $lockPath -Raw | ConvertFrom-Json
if ($runtimeLock.schemaVersion -ne 1) {
    throw "Unsupported runtime lock schema: $($runtimeLock.schemaVersion)"
}

New-Item -ItemType Directory -Force -Path $cacheRoot, $npmCache, $resourceRoot | Out-Null
$archivePath = Join-Path $cacheRoot $runtimeLock.node.archive
if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
    if ($Offline) {
        throw "Offline staging requires the cached Node archive: $archivePath"
    }
    Write-Host "Downloading Node.js $($runtimeLock.node.version)..."
    Invoke-WebRequest -Uri $runtimeLock.node.url -OutFile $archivePath -TimeoutSec 300
}

$actualArchiveHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualArchiveHash -ne $runtimeLock.node.sha256.ToLowerInvariant()) {
    throw "Node archive SHA-256 mismatch. Expected $($runtimeLock.node.sha256), got $actualArchiveHash."
}

$extractRoot = Assert-ProjectPath (Join-Path $cacheRoot 'node-extracted')
if (Test-Path -LiteralPath $extractRoot) {
    Remove-Item -LiteralPath $extractRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $extractRoot | Out-Null
Expand-Archive -LiteralPath $archivePath -DestinationPath $extractRoot -Force
$nodeDistributionRoot = Join-Path $extractRoot ("node-v$($runtimeLock.node.version)-$($runtimeLock.node.platform)")
foreach ($required in @('node.exe', 'LICENSE')) {
    if (-not (Test-Path -LiteralPath (Join-Path $nodeDistributionRoot $required) -PathType Leaf)) {
        throw "Node archive does not contain required file: $required"
    }
}

$nodeResourceRoot = Assert-ProjectPath $nodeResourceRoot
$hostResourceRoot = Assert-ProjectPath $hostResourceRoot
foreach ($target in @($nodeResourceRoot, $hostResourceRoot)) {
    if (Test-Path -LiteralPath $target) {
        Remove-Item -LiteralPath $target -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $target | Out-Null
}
Copy-Item -LiteralPath (Join-Path $nodeDistributionRoot 'node.exe') -Destination $nodeResourceRoot
Copy-Item -LiteralPath (Join-Path $nodeDistributionRoot 'LICENSE') -Destination $nodeResourceRoot
Copy-Item -LiteralPath $lockPath -Destination (Join-Path $nodeResourceRoot 'runtime.lock.json')

Copy-Item -LiteralPath (Join-Path $runtimeHostRoot 'package.json') -Destination $hostResourceRoot
Copy-Item -LiteralPath (Join-Path $runtimeHostRoot 'package-lock.json') -Destination $hostResourceRoot

Write-Host "Installing $($runtimeLock.dsh.package) $($runtimeLock.dsh.version) with npm ci..."
Push-Location $hostResourceRoot
try {
    $npmArguments = @('ci', '--omit=dev', '--no-audit', '--fund=false', '--cache', $npmCache)
    if ($Offline) {
        $npmArguments += '--offline'
    }
    & npm @npmArguments
    if ($LASTEXITCODE -ne 0) {
        throw "npm ci failed with exit code $LASTEXITCODE."
    }
}
finally {
    Pop-Location
}

Copy-Item -LiteralPath $lockPath -Destination (Join-Path $hostResourceRoot 'runtime.lock.json')
Copy-Item -LiteralPath (Join-Path $projectRoot 'THIRD_PARTY_NOTICES.md') -Destination $hostResourceRoot

# 从 npm lockfile 生成可审计的第三方版本与许可证清单。
$packageLock = Get-Content -LiteralPath (Join-Path $hostResourceRoot 'package-lock.json') -Raw | ConvertFrom-Json -AsHashtable
$licenseEntries = foreach ($property in $packageLock['packages'].GetEnumerator()) {
    if ([string]::IsNullOrWhiteSpace($property.Key) -or -not $property.Value['version']) {
        continue
    }
    $name = ($property.Key -split 'node_modules/')[-1]
    [pscustomobject][ordered]@{
        name = $name
        version = $property.Value['version']
        license = if ($property.Value['license']) { $property.Value['license'] } else { 'UNKNOWN' }
        integrity = $property.Value['integrity']
    }
}
$licenseEntries = @($licenseEntries | Sort-Object name, version -Unique)
$licenseManifestPath = Join-Path $hostResourceRoot 'third-party-licenses.json'
$licenseEntries | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $licenseManifestPath -Encoding utf8NoBOM

& (Join-Path $PSScriptRoot 'verify-runtime.ps1') -ResourceRoot $resourceRoot -ArchivePath $archivePath
if ($LASTEXITCODE -ne 0) {
    throw "Runtime verification failed with exit code $LASTEXITCODE."
}

Write-Host "Self-contained runtime staged at $resourceRoot"
