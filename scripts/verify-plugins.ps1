param(
    [string]$ResourceRoot = '..\src-tauri\resources\plugins'
)

$ErrorActionPreference = 'Stop'

$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$resourceRootPath = if ([System.IO.Path]::IsPathRooted($ResourceRoot)) {
    [System.IO.Path]::GetFullPath($ResourceRoot)
}
else {
    [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot $ResourceRoot))
}
$trackedLock = Join-Path $projectRoot 'plugins.lock.json'
$resourceLock = Join-Path $resourceRootPath 'plugins.lock.json'
if (-not (Test-Path -LiteralPath $resourceLock -PathType Leaf)) {
    throw "Staged plugin lock is missing: $resourceLock"
}
if ((Get-FileHash -LiteralPath $trackedLock -Algorithm SHA256).Hash -ne (Get-FileHash -LiteralPath $resourceLock -Algorithm SHA256).Hash) {
    throw 'Staged plugins.lock.json does not match the tracked lock file.'
}

$lock = Get-Content -LiteralPath $resourceLock -Raw | ConvertFrom-Json
$expectedOrder = @('dsh-at-file', '@omdsh-dev/dsh-genui', 'dsh-better-sidebar', '@linxin666/dsh-skins')
if (($lock.plugins.package -join '|') -ne ($expectedOrder -join '|')) {
    throw "Managed plugin order is invalid: $($lock.plugins.package -join ', ')"
}

foreach ($plugin in $lock.plugins) {
    $root = Join-Path $resourceRootPath ('node_modules\' + $plugin.package.Replace('/', '\'))
    $manifestPath = Join-Path $root 'package.json'
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "Plugin manifest is missing: $manifestPath"
    }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ($manifest.name -ne $plugin.package -or $manifest.version -ne $plugin.version) {
        throw "Plugin version mismatch for $($plugin.package): $($manifest.name) $($manifest.version)"
    }
    if (-not $manifest.dsh.bundle.patch) {
        throw "Plugin bundle patch is missing from manifest: $($plugin.package)"
    }
    foreach ($required in $plugin.requiredFiles) {
        if (-not (Test-Path -LiteralPath (Join-Path $root $required))) {
            throw "Plugin required file is missing: $($plugin.package) / $required"
        }
    }
}

# 插件必须复用内置 DSH 的完整 peer closure，禁止夹带第二套官方包。
$deepseekCopies = Join-Path $resourceRootPath 'node_modules\@deepseek-ai'
if (Test-Path -LiteralPath $deepseekCopies) {
    throw "Plugin runtime must not contain @deepseek-ai package copies: $deepseekCopies"
}

$packageLock = Get-Content -LiteralPath (Join-Path $resourceRootPath 'package-lock.json') -Raw | ConvertFrom-Json -AsHashtable
foreach ($plugin in $lock.plugins | Where-Object { $_.source.type -eq 'npm' }) {
    $entry = $packageLock['packages']['node_modules/' + $plugin.package]
    if ($entry['version'] -ne $plugin.version -or $entry['integrity'] -ne $plugin.source.integrity) {
        throw "npm lock entry does not match plugins.lock.json: $($plugin.package)"
    }
}
foreach ($dependency in $lock.transitivePackages) {
    $entry = $packageLock['packages']['node_modules/' + $dependency.package]
    if ($entry['version'] -ne $dependency.version -or $entry['integrity'] -ne $dependency.integrity) {
        throw "Transitive plugin lock entry mismatch: $($dependency.package)"
    }
}

$ptyRoot = Join-Path $resourceRootPath 'node_modules\node-pty'
$ptyBinary = Get-ChildItem -LiteralPath (Join-Path $ptyRoot 'prebuilds\win32-x64') -File -Filter 'pty.node' | Select-Object -First 1
if (-not $ptyBinary) {
    throw 'Better Sidebar Windows x64 PTY prebuild is missing.'
}
$nodePath = Join-Path $projectRoot 'src-tauri\resources\node\node.exe'
if (Test-Path -LiteralPath $nodePath -PathType Leaf) {
    & $nodePath -e "require(process.argv[1]); console.log('node-pty load ok')" $ptyRoot
    if ($LASTEXITCODE -ne 0) {
        throw 'Bundled Node could not load Better Sidebar node-pty.'
    }
}

$licenseManifest = @(Get-Content -LiteralPath (Join-Path $resourceRootPath 'third-party-licenses.json') -Raw | ConvertFrom-Json)
if ($licenseManifest.Count -eq 0) {
    throw 'Plugin third-party license manifest is empty.'
}
Write-Host "Plugins valid: $($lock.plugins.Count) managed bundles, PTY $($ptyBinary.Name), $($licenseManifest.Count) licensed packages."
