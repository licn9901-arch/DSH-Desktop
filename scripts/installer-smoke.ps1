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
$bundledNode = Join-Path $installRoot 'node\node.exe'
$bundledCli = Join-Path $installRoot 'host\node_modules\@deepseek-ai\dsh\lib\bin.js'
$bundledMarket = Join-Path $installRoot 'host\node_modules\dshmarket\package.json'
$bundledPnpm = Join-Path $installRoot 'host\node_modules\.bin\pnpm.cmd'
$bundledMarketPolicy = Join-Path $installRoot 'policy\dsh-market.patch.yml'
$bundledRuntimeLicenses = @(
    (Join-Path $installRoot 'node\LICENSE'),
    (Join-Path $installRoot 'host\node_modules\dshmarket\LICENSE'),
    (Join-Path $installRoot 'host\node_modules\pnpm\LICENSE'),
    (Join-Path $installRoot 'host\THIRD_PARTY_NOTICES.md')
)
$bundledPluginLock = Join-Path $installRoot 'plugins\plugins.lock.json'
$bundledPlugins = @(
    (Join-Path $installRoot 'plugins\node_modules\dsh-at-file\lib\index.js'),
    (Join-Path $installRoot 'plugins\node_modules\@omdsh-dev\dsh-genui\lib\assets\mermaid.js'),
    (Join-Path $installRoot 'plugins\node_modules\@omdsh-dev\dsh-genui\SKILL.md'),
    (Join-Path $installRoot 'plugins\node_modules\dsh-better-sidebar\lib\index.js'),
    (Join-Path $installRoot 'plugins\node_modules\@dsh-desktop\theme-settings\lib\index.js'),
    (Join-Path $installRoot 'plugins\node_modules\@dsh-desktop\theme-settings\lib\client.js'),
    (Join-Path $installRoot 'plugins\node_modules\@linxin666\dsh-skins\cordis.patch.yml'),
    (Join-Path $installRoot 'plugins\node_modules\@linxin666\dsh-client-ui-skin-center\lib\index.js'),
    (Join-Path $installRoot 'plugins\node_modules\@vectorize-io\hindsight-coding-agents\dist\dsh.js'),
    (Join-Path $installRoot 'plugins\node_modules\@liustack\modlens\dsh\index.js'),
    (Join-Path $installRoot 'plugins\node_modules\@liustack\modlens\dsh\client.js'),
    (Join-Path $installRoot 'plugins\node_modules\@zebbkira\dsh-skills-mcp-manager\lib\index.js'),
    (Join-Path $installRoot 'plugins\node_modules\@zebbkira\dsh-skills-mcp-manager\lib\client.js')
)
$webIndexCandidates = @(
    (Join-Path $installRoot 'host\node_modules\@deepseek-ai\dsh-web-frontend\dist\index.html'),
    (Join-Path $installRoot 'host\node_modules\@deepseek-ai\dsh\node_modules\@deepseek-ai\dsh-web-frontend\dist\index.html')
)

if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) {
    throw "Installer not found: $installerPath"
}
if ((Test-Path -LiteralPath $installedExe -PathType Leaf) -or
    (Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
    throw "Refusing to overwrite an existing installation: $installRoot"
}

$installed = $false
try {
    # 自动化使用 NSIS 静默安装；应用生命周期仍由完整桌面冒烟脚本验证。
    $installProcess = Start-Process -FilePath $installerPath -ArgumentList '/S' -PassThru -Wait
    $installed = Test-Path -LiteralPath $installRoot -PathType Container
    if ($installProcess.ExitCode -ne 0) {
        throw "Silent installation failed with exit code $($installProcess.ExitCode)."
    }

    # NSIS 可能把实际复制交给后台子进程；所有关键资源落盘后才能启动应用。
    $installDeadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        $installed = Test-Path -LiteralPath $installRoot -PathType Container
        $webReady = $webIndexCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
        $installReady = (Test-Path -LiteralPath $installedExe -PathType Leaf) -and
            (Test-Path -LiteralPath $uninstaller -PathType Leaf) -and
            (Test-Path -LiteralPath $bundledNode -PathType Leaf) -and
            (Test-Path -LiteralPath $bundledCli -PathType Leaf) -and
            (Test-Path -LiteralPath $bundledMarket -PathType Leaf) -and
            (Test-Path -LiteralPath $bundledPnpm -PathType Leaf) -and
            (Test-Path -LiteralPath $bundledMarketPolicy -PathType Leaf) -and
            (($bundledRuntimeLicenses | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf }).Count -eq $bundledRuntimeLicenses.Count) -and
            (Test-Path -LiteralPath $bundledPluginLock -PathType Leaf) -and
            (($bundledPlugins | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf }).Count -eq $bundledPlugins.Count) -and
            $webReady
        if (-not $installReady) {
            Start-Sleep -Milliseconds 250
        }
    } while (-not $installReady -and (Get-Date) -lt $installDeadline)
    if (-not $installReady) {
        throw "Installation did not finish writing the bundled runtime within $TimeoutSeconds seconds."
    }

    & (Join-Path $PSScriptRoot 'smoke-test.ps1') `
        -Exe $installedExe `
        -TimeoutSeconds $TimeoutSeconds `
        -UseBundledRuntime
}
finally {
    if ($installed) {
        $uninstallerDeadline = (Get-Date).AddSeconds(60)
        while (-not (Test-Path -LiteralPath $uninstaller -PathType Leaf) -and (Get-Date) -lt $uninstallerDeadline) {
            Start-Sleep -Milliseconds 250
        }
    }
    if ($installed -and (Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
        $uninstallProcess = Start-Process -FilePath $uninstaller -ArgumentList '/S' -PassThru -Wait
        if ($uninstallProcess.ExitCode -ne 0) {
            throw "Silent uninstall failed with exit code $($uninstallProcess.ExitCode)."
        }
        $uninstallDeadline = (Get-Date).AddSeconds(60)
        while ((Test-Path -LiteralPath $installedExe) -and (Get-Date) -lt $uninstallDeadline) {
            Start-Sleep -Milliseconds 250
        }
    }
}

if (Test-Path -LiteralPath $installedExe) {
    throw "Installed executable remained after uninstall: $installedExe"
}

Write-Host 'INSTALLER SMOKE OK: install, bundled runtime, lifecycle and uninstall verified.'
