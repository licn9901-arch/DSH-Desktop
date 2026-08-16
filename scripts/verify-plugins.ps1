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
$expectedOrder = @(
    'dsh-at-file',
    '@omdsh-dev/dsh-genui',
    'dsh-better-sidebar',
    '@dsh-desktop/theme-settings',
    '@linxin666/dsh-skins',
    '@vectorize-io/hindsight-coding-agents',
    '@liustack/modlens',
    '@zebbkira/dsh-skills-mcp-manager'
)
if (($lock.plugins.package -join '|') -ne ($expectedOrder -join '|')) {
    throw "Managed plugin order is invalid: $($lock.plugins.package -join ', ')"
}

$themeClient = Join-Path $resourceRootPath 'node_modules\@dsh-desktop\theme-settings\lib\client.js'
$themeSource = Get-Content -LiteralPath $themeClient -Raw
foreach ($marker in @('id: "desktop-theme"', '"web-ui.plugin.item"', 'renderSlot("web-ui.plugin.item"')) {
    if (-not $themeSource.Contains($marker)) {
        throw "Desktop theme adapter is missing required client marker: $marker"
    }
}
$themeHost = Get-Content -LiteralPath (Join-Path $resourceRootPath 'node_modules\@dsh-desktop\theme-settings\lib\index.js') -Raw
foreach ($marker in @('/api/desktop-managed-plugins', 'PROTECTED_BUNDLES', 'atomicWriteProfile', 'serializeWrite')) {
    if (-not $themeHost.Contains($marker)) {
        throw "Desktop managed-plugin API is missing required host marker: $marker"
    }
}

foreach ($skill in $lock.skills) {
    $source = Join-Path $resourceRootPath (
        'node_modules\' + $skill.sourcePackage.Replace('/', '\') + '\' + $skill.sourceFile
    )
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "Managed Skill source is missing: $source"
    }
    $actual = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $skill.sha256.ToLowerInvariant()) {
        throw "Managed Skill SHA-256 mismatch for $($skill.name). Expected $($skill.sha256), got $actual."
    }
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
$deepseekCopies = @(Get-ChildItem -LiteralPath (Join-Path $resourceRootPath 'node_modules') -Directory -Recurse -Force |
    Where-Object { $_.Name -eq '@deepseek-ai' })
if ($deepseekCopies.Count -gt 0) {
    throw "Plugin runtime must not contain @deepseek-ai package copies: $($deepseekCopies[0].FullName)"
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
$licensedNames = @($licenseManifest | ForEach-Object { $_.name })
foreach ($plugin in $lock.plugins) {
    if ($plugin.package -notin $licensedNames) {
        throw "Plugin is missing from third-party license manifest: $($plugin.package)"
    }
}
Write-Host "Plugins valid: $($lock.plugins.Count) managed bundles, $($lock.skills.Count) managed Skills, PTY $($ptyBinary.Name), $($licenseManifest.Count) licensed packages."
