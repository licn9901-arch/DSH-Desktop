param(
    [string]$InstallerDirectory = '..\src-tauri\target\release\bundle\nsis'
)

$ErrorActionPreference = 'Stop'

$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$installerRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot $InstallerDirectory))
$deployRoot = Join-Path $projectRoot '.deploy-artifacts'
$runtimeLock = Get-Content -LiteralPath (Join-Path $projectRoot 'runtime.lock.json') -Raw | ConvertFrom-Json
$pluginLock = Get-Content -LiteralPath (Join-Path $projectRoot 'plugins.lock.json') -Raw | ConvertFrom-Json
$package = Get-Content -LiteralPath (Join-Path $projectRoot 'package.json') -Raw | ConvertFrom-Json
$installerPattern = "*_$($package.version)_x64-setup.exe"
$installers = @(Get-ChildItem -LiteralPath $installerRoot -Filter $installerPattern -File)

if ($installers.Count -ne 1) {
    throw "Expected exactly one $installerPattern installer in $installerRoot, found $($installers.Count)."
}
$installer = $installers[0]

# 发布目录只能位于仓库内，避免清理错误路径。
$deployRoot = [System.IO.Path]::GetFullPath($deployRoot)
$projectPrefix = $projectRoot.TrimEnd('\') + '\'
if (-not $deployRoot.StartsWith($projectPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to write release artifacts outside the project: $deployRoot"
}
if (Test-Path -LiteralPath $deployRoot) {
    Remove-Item -LiteralPath $deployRoot -Recurse -Force
}
New-Item -ItemType Directory -Path $deployRoot | Out-Null

$publishedInstallerName = $installer.Name.Replace(' ', '.')
$publishedInstaller = Join-Path $deployRoot $publishedInstallerName
Copy-Item -LiteralPath $installer.FullName -Destination $publishedInstaller
$hash = (Get-FileHash -LiteralPath $publishedInstaller -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  $publishedInstallerName" | Set-Content -LiteralPath "$publishedInstaller.sha256" -Encoding ascii

$licenseSource = Join-Path $projectRoot 'src-tauri\resources\host\third-party-licenses.json'
if (-not (Test-Path -LiteralPath $licenseSource -PathType Leaf)) {
    throw "Third-party license manifest is missing: $licenseSource"
}
Copy-Item -LiteralPath $licenseSource -Destination (Join-Path $deployRoot 'third-party-licenses.json')
$pluginLicenseSource = Join-Path $projectRoot 'src-tauri\resources\plugins\third-party-licenses.json'
if (-not (Test-Path -LiteralPath $pluginLicenseSource -PathType Leaf)) {
    throw "Plugin license manifest is missing: $pluginLicenseSource"
}
Copy-Item -LiteralPath $pluginLicenseSource -Destination (Join-Path $deployRoot 'plugin-third-party-licenses.json')
Copy-Item -LiteralPath (Join-Path $projectRoot 'plugins.lock.json') -Destination $deployRoot
Copy-Item -LiteralPath (Join-Path $projectRoot 'THIRD_PARTY_NOTICES.md') -Destination $deployRoot

$sizeMiB = [Math]::Round($installer.Length / 1MB, 2)
@"
# Build Summary

- Version: $($package.version)
- Target: Windows x64 NSIS
- Node.js: $($runtimeLock.node.version)
- @deepseek-ai/dsh: $($runtimeLock.dsh.version)
- Managed plugins: $(($pluginLock.plugins | ForEach-Object { "$($_.package)@$($_.version)" }) -join ', ')
- Installer: $publishedInstallerName
- Installer size: $($installer.Length) bytes ($sizeMiB MiB)
- SHA-256: $hash
- Authenticode: unsigned preview
- Generated at: $([DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ'))
"@ | Set-Content -LiteralPath (Join-Path $deployRoot 'build-summary.md') -Encoding utf8NoBOM

Write-Host "Release artifacts written to $deployRoot"
