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

Write-Host 'INSTALLER ISOLATION TESTS OK'
