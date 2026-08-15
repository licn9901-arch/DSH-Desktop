param(
    [Parameter(Mandatory = $true)]
    [string]$Installer,
    [int]$TimeoutSeconds = 180
)

$ErrorActionPreference = 'Stop'

$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$installerPath = if ([System.IO.Path]::IsPathRooted($Installer)) {
    [System.IO.Path]::GetFullPath($Installer)
}
else {
    [System.IO.Path]::GetFullPath((Join-Path $projectRoot $Installer))
}
$installRoot = Join-Path $env:LOCALAPPDATA 'DeepSeek Harness Desktop'
$installedExe = Join-Path $installRoot 'dsh-desktop.exe'
$uninstaller = Join-Path $installRoot 'uninstall.exe'

if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) {
    throw "Installer not found: $installerPath"
}
if (Test-Path -LiteralPath $installRoot) {
    throw "Refusing to overwrite an existing installation: $installRoot"
}

$installed = $false
try {
    # CI 使用 NSIS 静默安装；应用生命周期仍由完整桌面冒烟脚本验证。
    $installProcess = Start-Process -FilePath $installerPath -ArgumentList '/S' -PassThru -Wait
    if ($installProcess.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $installedExe -PathType Leaf)) {
        throw "Silent installation failed with exit code $($installProcess.ExitCode)."
    }
    $installed = $true

    & (Join-Path $PSScriptRoot 'smoke-test.ps1') `
        -Exe $installedExe `
        -TimeoutSeconds $TimeoutSeconds `
        -UseBundledRuntime
    if ($LASTEXITCODE -ne 0) {
        throw "Installed application smoke failed with exit code $LASTEXITCODE."
    }
}
finally {
    if ($installed -and (Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
        $uninstallProcess = Start-Process -FilePath $uninstaller -ArgumentList '/S' -PassThru -Wait
        if ($uninstallProcess.ExitCode -ne 0) {
            throw "Silent uninstall failed with exit code $($uninstallProcess.ExitCode)."
        }
    }
}

if (Test-Path -LiteralPath $installedExe) {
    throw "Installed executable remained after uninstall: $installedExe"
}

Write-Host 'INSTALLER SMOKE OK: install, bundled runtime, lifecycle and uninstall verified.'
