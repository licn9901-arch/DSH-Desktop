$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'release-installer-isolation.ps1')

$systemTemp = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$ownedRoot = Join-Path $systemTemp 'dsh-desktop-installer-smoke-123456789abc\install'
$ownedExe = Join-Path $ownedRoot 'dsh-desktop.exe'
$quotedOwnedExe = '"' + $ownedExe + '"'
$siblingRoot = Join-Path $systemTemp 'dsh-desktop-installer-smoke-123456789abd\install'
$untrustedRoot = Join-Path $systemTemp 'untrusted-installer-root\install'

# 合法临时根及其子文件必须被识别为本轮所有，带引号的注册表路径也应正常处理。
Assert-DshInstallerTestRoots -OwnedInstallRoots @($ownedRoot)
if (-not (Test-DshInstallerOwnedPath -Path $ownedExe -OwnedInstallRoots @($ownedRoot))) {
    throw 'Owned installer path was rejected.'
}
if (-not (Test-DshInstallerOwnedPath -Path $quotedOwnedExe -OwnedInstallRoots @($ownedRoot))) {
    throw 'Quoted owned installer path was rejected.'
}

# 相似前缀和普通临时目录都不能绕过删除边界。
if (Test-DshInstallerOwnedPath -Path (Join-Path $siblingRoot 'dsh-desktop.exe') -OwnedInstallRoots @($ownedRoot)) {
    throw 'Sibling installer path bypassed ownership validation.'
}
$rejected = $false
try {
    Assert-DshInstallerTestRoots -OwnedInstallRoots @($untrustedRoot)
} catch {
    $rejected = $true
}
if (-not $rejected) {
    throw 'Untrusted installer root bypassed temp-prefix validation.'
}

# 删除辅助函数只能接受系统临时目录的直接子目录，嵌套或相似路径必须拒绝。
$nestedRoot = Join-Path $systemTemp 'parent\dsh-desktop-installer-smoke-123456789abc'
$rejected = $false
try {
    Remove-DshInstallerTestDirectory -Root $nestedRoot
} catch {
    $rejected = $true
}
if (-not $rejected) {
    throw 'Nested installer root bypassed deletion validation.'
}

# legacy NSIS 对深层资源仍受 MAX_PATH 约束，合法前缀不能绕过路径预算。
$longName = 'dsh-desktop-installer-smoke-' + ('a' * 80)
$longRoot = Join-Path (Join-Path $systemTemp $longName) 'install'
$rejected = $false
try {
    Assert-DshInstallerTestRoots -OwnedInstallRoots @($longRoot)
} catch {
    $rejected = $true
}
if (-not $rejected) {
    throw 'Overlong legacy installer root bypassed the path budget.'
}

$generatedRoot = New-DshInstallerTestRoot
Assert-DshInstallerTestRoots -OwnedInstallRoots @((Join-Path $generatedRoot 'install'))
if (([System.IO.Path]::GetFileName($generatedRoot)).Length -ne 40) {
    throw 'Generated installer test root is not using the bounded short name.'
}

# payload 必须使用结构化 phase；固定 preview.7 才允许解析旧 host ready 行。
$phaseLog = 'phase=core_ready duration_ms=4321 attempt=1'
if ((Get-DshDesktopReadyDuration -Content $phaseLog) -ne 4321) {
    throw 'Structured readiness duration was not parsed.'
}
$legacyLog = 'host ready: http://127.0.0.1:61743 (started in 64678 ms)'
if ($null -ne (Get-DshDesktopReadyDuration -Content $legacyLog)) {
    throw 'Legacy readiness format was accepted without explicit opt-in.'
}
if ((Get-DshDesktopReadyDuration -Content $legacyLog -AllowLegacyFormat) -ne 64678) {
    throw 'Legacy readiness duration was not parsed after explicit opt-in.'
}

# 安装版 WebView2 数据目录随隔离安装根清理；当前 WebView2 对显式测试目录可能阻塞初始化。
$smokeScript = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'smoke-test.ps1') -Raw
$installerSmokeScript = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'installer-smoke.ps1') -Raw
$upgradeMatrixScript = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'upgrade-matrix.ps1') -Raw
$benchmarkScript = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'benchmark-compare.ps1') -Raw
if ($smokeScript -notmatch '\[switch\]\$UseInstalledWebViewDataDirectory') {
    throw 'Smoke test does not expose the installed WebView data-directory mode.'
}
if ($smokeScript -notmatch '\$navigationTimeoutSeconds\s*=\s*\[Math\]::Min\(60,') {
    throw 'Smoke test does not allow the installed WebView startup up to 60 seconds.'
}
if ($installerSmokeScript -notmatch 'UseInstalledWebViewDataDirectory\s*=\s*\$true') {
    throw 'Installer smoke does not select the installed WebView data-directory mode.'
}
if ($upgradeMatrixScript -notmatch 'UseInstalledWebViewDataDirectory\s*=\s*\$true') {
    throw 'Upgrade matrix does not select the installed WebView data-directory mode.'
}
foreach ($releaseScript in @(
    @{ Name = 'installer smoke'; Content = $installerSmokeScript },
    @{ Name = 'upgrade matrix'; Content = $upgradeMatrixScript },
    @{ Name = 'startup benchmark'; Content = $benchmarkScript }
)) {
    if ($releaseScript.Content -match '\$env:DSH_DESKTOP_WEBVIEW_TEST_DATA_DIR\s*=') {
        throw "$($releaseScript.Name) still injects the WebView test data-directory override."
    }
    if ($releaseScript.Content -notmatch 'Invoke-DshSilentUninstall') {
        throw "$($releaseScript.Name) does not use the bounded NSIS uninstall retry helper."
    }
}

Write-Host 'INSTALLER ISOLATION TESTS OK'
