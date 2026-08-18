param(
    [switch]$Offline
)

$ErrorActionPreference = 'Stop'

$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$lockPath = Join-Path $projectRoot 'plugins.lock.json'
$runtimeRoot = Join-Path $projectRoot 'plugin-runtime'
$cacheRoot = Join-Path $projectRoot '.runtime-cache\plugins'
$npmCache = Join-Path $projectRoot '.runtime-cache\plugin-npm-cache'
$resourceRoot = Join-Path $projectRoot 'src-tauri\resources\plugins'

# 删除或覆盖前验证目标位于仓库，避免路径变量异常扩大影响范围。
function Assert-ProjectPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    $resolved = [System.IO.Path]::GetFullPath($Path)
    $prefix = $projectRoot.TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to modify a path outside the project: $resolved"
    }
    return $resolved
}

# 下载 GitHub tag 归档并严格比对锁文件 SHA-256；离线模式只读缓存。
function Get-LockedArchive {
    param([Parameter(Mandatory = $true)]$Plugin)

    $source = $Plugin.source
    $archivePath = Join-Path $cacheRoot $source.archive
    if (-not (Test-Path -LiteralPath $archivePath -PathType Leaf)) {
        $researchCache = Join-Path $projectRoot ('.runtime-cache\plugin-research\' + $source.archive)
        if (Test-Path -LiteralPath $researchCache -PathType Leaf) {
            Copy-Item -LiteralPath $researchCache -Destination $archivePath
        }
        elseif ($Offline) {
            throw "Offline staging requires cached archive: $archivePath"
        }
        else {
            Write-Host "Downloading $($Plugin.package) $($Plugin.version)..."
            Invoke-WebRequest -Uri $source.url -OutFile $archivePath -Headers @{ 'User-Agent' = 'DSH-Desktop-build' } -TimeoutSec 300
        }
    }
    $actual = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $source.sha256.ToLowerInvariant()) {
        throw "Archive SHA-256 mismatch for $($Plugin.package). Expected $($source.sha256), got $actual."
    }
    return $archivePath
}

# 从单根目录 tarball 复制预构建发布内容，不运行仓库中的任何脚本。
function Expand-LockedPlugin {
    param(
        [Parameter(Mandatory = $true)]$Plugin,
        [Parameter(Mandatory = $true)][string]$ArchivePath
    )

    $safeName = $Plugin.package.Replace('@', '').Replace('/', '-')
    $extractRoot = Assert-ProjectPath (Join-Path $cacheRoot ("extract-$safeName"))
    if (Test-Path -LiteralPath $extractRoot) {
        Remove-Item -LiteralPath $extractRoot -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $extractRoot | Out-Null
    & tar.exe -xzf $ArchivePath -C $extractRoot
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to extract $ArchivePath."
    }
    $roots = @(Get-ChildItem -LiteralPath $extractRoot -Directory)
    if ($roots.Count -ne 1) {
        throw "Plugin archive must contain exactly one root directory: $ArchivePath"
    }
    $target = Join-Path $resourceRoot ('node_modules\' + $Plugin.package.Replace('/', '\'))
    New-Item -ItemType Directory -Force -Path $target | Out-Null
    Copy-Item -Path (Join-Path $roots[0].FullName '*') -Destination $target -Recurse -Force
}

if (-not (Test-Path -LiteralPath $lockPath -PathType Leaf)) {
    throw "Plugin lock file is missing: $lockPath"
}
$pluginLock = Get-Content -LiteralPath $lockPath -Raw | ConvertFrom-Json
if ($pluginLock.schemaVersion -ne 2) {
    throw "Unsupported plugin lock schema: $($pluginLock.schemaVersion)"
}

New-Item -ItemType Directory -Force -Path $cacheRoot, $npmCache | Out-Null
$resourceRoot = Assert-ProjectPath $resourceRoot
if (Test-Path -LiteralPath $resourceRoot) {
    Remove-Item -LiteralPath $resourceRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $resourceRoot | Out-Null
Copy-Item -LiteralPath $lockPath -Destination (Join-Path $resourceRoot 'plugins.lock.json')
$storeDigest = (Get-FileHash -LiteralPath $lockPath -Algorithm SHA256).Hash.ToLowerInvariant().Substring(0, 16)
$storeDigest | Set-Content -LiteralPath (Join-Path $resourceRoot 'store.digest') -Encoding ascii -NoNewline
Copy-Item -LiteralPath (Join-Path $runtimeRoot 'package.json') -Destination $resourceRoot
Copy-Item -LiteralPath (Join-Path $runtimeRoot 'package-lock.json') -Destination $resourceRoot

Write-Host 'Installing locked npm plugin dependencies with lifecycle scripts disabled...'
Push-Location $resourceRoot
try {
    $npmArguments = @('ci', '--omit=dev', '--ignore-scripts', '--legacy-peer-deps', '--no-audit', '--fund=false', '--cache', $npmCache)
    if ($Offline) {
        $npmArguments += '--offline'
    }
    & npm @npmArguments
    if ($LASTEXITCODE -ne 0) {
        throw "Plugin npm ci failed with exit code $LASTEXITCODE."
    }
}
finally {
    Pop-Location
}

foreach ($plugin in $pluginLock.plugins) {
    if ($plugin.source.type -eq 'local') {
        $source = Assert-ProjectPath (Join-Path $projectRoot $plugin.source.path)
        if (-not (Test-Path -LiteralPath $source -PathType Container)) {
            throw "Local plugin source is missing: $source"
        }
        $target = Assert-ProjectPath (Join-Path $resourceRoot ('node_modules\' + $plugin.package.Replace('/', '\')))
        New-Item -ItemType Directory -Force -Path $target | Out-Null
        Copy-Item -Path (Join-Path $source '*') -Destination $target -Recurse -Force
        continue
    }
    if ($plugin.source.type -notin @('github-tarball', 'github-release-asset')) {
        continue
    }
    $archivePath = Get-LockedArchive -Plugin $plugin
    Expand-LockedPlugin -Plugin $plugin -ArchivePath $archivePath
}

# 生成只包含实际交付包的许可证清单，便于发行物审计。
$packageLock = Get-Content -LiteralPath (Join-Path $resourceRoot 'package-lock.json') -Raw | ConvertFrom-Json -AsHashtable
$licenses = foreach ($property in $packageLock['packages'].GetEnumerator()) {
    if ([string]::IsNullOrWhiteSpace($property.Key) -or -not $property.Value['version']) {
        continue
    }
    [pscustomobject][ordered]@{
        name = ($property.Key -split 'node_modules/')[-1]
        version = $property.Value['version']
        license = if ($property.Value['license']) { $property.Value['license'] } else { 'UNKNOWN' }
        integrity = $property.Value['integrity']
    }
}
foreach ($plugin in $pluginLock.plugins | Where-Object { $_.source.type -in @('github-tarball', 'github-release-asset') }) {
    $licenses += [pscustomobject][ordered]@{
        name = $plugin.package
        version = $plugin.version
        license = $plugin.license
        integrity = 'sha256-' + $plugin.source.sha256
    }
}
foreach ($plugin in $pluginLock.plugins | Where-Object { $_.source.type -eq 'local' }) {
    $licenses += [pscustomobject][ordered]@{
        name = $plugin.package
        version = $plugin.version
        license = $plugin.license
        integrity = 'local-' + $plugin.source.path
    }
}
$licenses | Sort-Object name, version -Unique | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $resourceRoot 'third-party-licenses.json') -Encoding utf8NoBOM

& (Join-Path $PSScriptRoot 'verify-plugins.ps1') -ResourceRoot $resourceRoot
if ($LASTEXITCODE -ne 0) {
    throw "Plugin verification failed with exit code $LASTEXITCODE."
}
Write-Host "Managed plugins staged at $resourceRoot"
