$script:DshInstallerUninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\DeepSeek Harness Desktop'
$script:DshInstallerProductKey = 'HKCU:\Software\github\DeepSeek Harness Desktop'
$script:DshInstallerRunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$script:DshInstallerRunValue = 'DeepSeek Harness Desktop'
$script:DshInstallerTestRootPrefix = 'dsh-desktop-installer-smoke-'
$script:DshInstallerLegacyPathProbe = 'host\node_modules\@opentelemetry\sdk-logs\node_modules\@opentelemetry\resources\build\src\detectors\platform\node\machine-id\getMachineId.js'
$script:DshInstallerLegacyPathLimit = 240

# 创建短且可识别的系统临时目录，给不支持长路径的 legacy NSIS 保留足够路径预算。
function New-DshInstallerTestRoot {
    $name = $script:DshInstallerTestRootPrefix + [guid]::NewGuid().ToString('N').Substring(0, 12)
    return Join-Path ([System.IO.Path]::GetTempPath()) $name
}

# 读取桌面启动完成耗时；旧格式只用于固定 preview.7 基线，payload 必须提供结构化 phase 日志。
function Get-DshDesktopReadyDuration {
    param(
        [Parameter(Mandatory = $true)][string]$Content,
        [switch]$AllowLegacyFormat
    )
    $phaseMatch = [regex]::Match($Content, 'phase=core_ready duration_ms=(\d+) attempt=')
    if ($phaseMatch.Success) { return [int]$phaseMatch.Groups[1].Value }
    if ($AllowLegacyFormat) {
        $legacyMatch = [regex]::Match($Content, 'host ready: https?://[^\s]+ \(started in (\d+) ms\)')
        if ($legacyMatch.Success) { return [int]$legacyMatch.Groups[1].Value }
    }
    return $null
}

# 返回 Tauri 默认入口和项目自定义卸载入口可能使用的全部固定快捷方式路径。
function Get-DshInstallerShortcutPaths {
    $desktop = [Environment]::GetFolderPath('Desktop')
    $programs = [Environment]::GetFolderPath('Programs')
    return @(
        (Join-Path $desktop 'DeepSeek Harness Desktop.lnk'),
        (Join-Path $programs 'DeepSeek Harness Desktop\DeepSeek Harness Desktop.lnk'),
        (Join-Path $programs 'DeepSeek Harness Desktop\Uninstall DeepSeek Harness Desktop.lnk'),
        (Join-Path $programs 'DeepSeek Harness Desktop.lnk'),
        (Join-Path $programs 'Uninstall DeepSeek Harness Desktop.lnk')
    )
}

# 返回会被正式 NSIS 安装器复用的当前用户进程、注册表和快捷方式。
function Get-DshInstallerUserStateConflicts {
    $conflicts = @()
    $processes = @(Get-Process dsh-desktop -ErrorAction SilentlyContinue)
    if ($processes.Count -gt 0) {
        $conflicts += "running process PID $($processes.Id -join ', ')"
    }

    foreach ($key in @($script:DshInstallerUninstallKey, $script:DshInstallerProductKey)) {
        if (Test-Path -LiteralPath $key) {
            $conflicts += "registry key $key"
        }
    }
    if (Test-Path -LiteralPath $script:DshInstallerRunKey) {
        $runValue = (Get-Item -LiteralPath $script:DshInstallerRunKey).GetValue(
            $script:DshInstallerRunValue,
            $null
        )
        if ($null -ne $runValue) {
            $conflicts += "registry value $script:DshInstallerRunKey\$script:DshInstallerRunValue"
        }
    }

    foreach ($shortcut in Get-DshInstallerShortcutPaths) {
        if (Test-Path -LiteralPath $shortcut -PathType Leaf) {
            $conflicts += "shortcut $shortcut"
        }
    }
    return $conflicts
}

# 发布安装器会使用固定 HKCU 键和快捷方式名，因此矩阵只能在无既有安装状态的专用用户中运行。
function Assert-DshInstallerTestUserIsClean {
    $conflicts = @(Get-DshInstallerUserStateConflicts)
    if ($conflicts.Count -gt 0) {
        throw "Installer gates require a clean disposable Windows user; conflicts: $($conflicts -join '; ')."
    }
}

# 判断注册表或快捷方式目标是否属于本轮在系统临时目录创建的安装根。
function Test-DshInstallerOwnedPath {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string[]]$OwnedInstallRoots
    )
    if ([string]::IsNullOrWhiteSpace($Path)) { return $false }
    $candidate = $Path.Trim().Trim('"')
    try {
        $candidate = [System.IO.Path]::GetFullPath($candidate)
    } catch {
        return $false
    }
    foreach ($root in $OwnedInstallRoots) {
        $prefix = [System.IO.Path]::GetFullPath($root).TrimEnd('\') + '\'
        if ($candidate.Equals($prefix.TrimEnd('\'), [System.StringComparison]::OrdinalIgnoreCase) -or
            $candidate.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
            return $true
        }
    }
    return $false
}

# 验证所有自动化安装根都位于系统临时目录，并带有专用前缀。
function Assert-DshInstallerTestRoots {
    param([Parameter(Mandatory = $true)][string[]]$OwnedInstallRoots)
    $systemTempPath = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\')
    foreach ($root in $OwnedInstallRoots) {
        $resolved = [System.IO.Path]::GetFullPath($root)
        $testRoot = [System.IO.Path]::GetDirectoryName($resolved.TrimEnd('\'))
        if ($null -eq $testRoot -or
            -not ([System.IO.Path]::GetDirectoryName($testRoot)).Equals(
                $systemTempPath,
                [System.StringComparison]::OrdinalIgnoreCase
            ) -or
            -not ([System.IO.Path]::GetFileName($testRoot)).StartsWith(
                $script:DshInstallerTestRootPrefix,
                [System.StringComparison]::OrdinalIgnoreCase
            )) {
            throw "Installer test roots must use the system temp isolation prefix: $resolved"
        }
        $legacyProbe = Join-Path $resolved $script:DshInstallerLegacyPathProbe
        if ($legacyProbe.Length -gt $script:DshInstallerLegacyPathLimit) {
            throw "Installer test root exceeds the legacy NSIS path budget ($($legacyProbe.Length) > $script:DshInstallerLegacyPathLimit): $resolved"
        }
    }
}

# 先验证 NSIS 默认自复制卸载；若 CI/Job 回收了子进程，则用 _?= 在当前进程完成同一卸载逻辑。
function Invoke-DshSilentUninstall {
    param(
        [Parameter(Mandatory = $true)][string]$Uninstaller,
        [Parameter(Mandatory = $true)][string[]]$CompletionPaths,
        [int]$TimeoutSeconds = 60
    )
    if ($CompletionPaths.Count -eq 0) { throw 'Silent uninstall requires at least one completion path.' }

    $installRoot = [System.IO.Path]::GetDirectoryName([System.IO.Path]::GetFullPath($Uninstaller))
    foreach ($attempt in 1..2) {
        if (-not (Test-Path -LiteralPath $Uninstaller -PathType Leaf)) {
            if (@($CompletionPaths | Where-Object { Test-Path -LiteralPath $_ }).Count -eq 0) { return }
            throw "Uninstaller disappeared before cleanup completed: $Uninstaller"
        }

        $arguments = if ($attempt -eq 1) {
            @('/S')
        }
        else {
            # _?= 必须是最后一个参数；NSIS 会跳过临时自复制，父进程因而能可靠等待卸载完成。
            @('/S', "_?=$installRoot")
        }
        $process = Start-Process -FilePath $Uninstaller -ArgumentList $arguments -PassThru
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
            throw "Silent uninstall timed out on attempt ${attempt}: $Uninstaller"
        }
        if ($process.ExitCode -ne 0) {
            throw "Silent uninstall failed with code $($process.ExitCode) on attempt ${attempt}: $Uninstaller"
        }

        $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
        do {
            $remaining = @($CompletionPaths | Where-Object { Test-Path -LiteralPath $_ })
            if ($remaining.Count -eq 0) { return }
            Start-Sleep -Milliseconds 250
        } while ((Get-Date) -lt $deadline)

        if ($attempt -eq 1) {
            Write-Warning "Silent uninstall child made no cleanup progress; retrying in direct NSIS mode: $Uninstaller"
        }
    }

    $remaining = @($CompletionPaths | Where-Object { Test-Path -LiteralPath $_ })
    throw "Silent uninstall left managed paths after default and direct attempts: $($remaining -join '; ')"
}

# 清理由干净测试用户中的安装器场景创建的固定 Shell 状态；遇到非本轮目标时立即拒绝删除。
function Clear-DshInstallerTestUserState {
    param([Parameter(Mandatory = $true)][string[]]$OwnedInstallRoots)
    if ($OwnedInstallRoots.Count -eq 0) { return }

    Assert-DshInstallerTestRoots -OwnedInstallRoots $OwnedInstallRoots

    if (Test-Path -LiteralPath $script:DshInstallerUninstallKey) {
        $key = Get-Item -LiteralPath $script:DshInstallerUninstallKey
        $location = [string]$key.GetValue('InstallLocation', '')
        if (-not (Test-DshInstallerOwnedPath -Path $location -OwnedInstallRoots $OwnedInstallRoots)) {
            throw "Refusing to remove an uninstall key not owned by this test: $location"
        }
        Remove-Item -LiteralPath $script:DshInstallerUninstallKey -Recurse -Force
    }

    if (Test-Path -LiteralPath $script:DshInstallerProductKey) {
        $key = Get-Item -LiteralPath $script:DshInstallerProductKey
        $location = [string]$key.GetValue('', '')
        if (-not (Test-DshInstallerOwnedPath -Path $location -OwnedInstallRoots $OwnedInstallRoots)) {
            throw "Refusing to remove a product key not owned by this test: $location"
        }
        Remove-Item -LiteralPath $script:DshInstallerProductKey -Recurse -Force
    }

    if (Test-Path -LiteralPath $script:DshInstallerRunKey) {
        $runKey = Get-Item -LiteralPath $script:DshInstallerRunKey
        $command = [string]$runKey.GetValue($script:DshInstallerRunValue, '')
        if (-not [string]::IsNullOrWhiteSpace($command)) {
            $owned = @($OwnedInstallRoots | Where-Object {
                $command.Contains([System.IO.Path]::GetFullPath($_), [System.StringComparison]::OrdinalIgnoreCase)
            })
            if ($owned.Count -eq 0) {
                throw "Refusing to remove an autostart value not owned by this test: $command"
            }
            Remove-ItemProperty -LiteralPath $script:DshInstallerRunKey -Name $script:DshInstallerRunValue -Force
        }
    }

    $shell = New-Object -ComObject WScript.Shell
    try {
        foreach ($shortcut in Get-DshInstallerShortcutPaths) {
            if (-not (Test-Path -LiteralPath $shortcut -PathType Leaf)) { continue }
            $target = $shell.CreateShortcut($shortcut).TargetPath
            if (-not (Test-DshInstallerOwnedPath -Path $target -OwnedInstallRoots $OwnedInstallRoots)) {
                throw "Refusing to remove a shortcut not owned by this test: $shortcut -> $target"
            }
            Remove-Item -LiteralPath $shortcut -Force
        }
    } finally {
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell)
    }

    $startMenuFolder = Join-Path ([Environment]::GetFolderPath('Programs')) 'DeepSeek Harness Desktop'
    if (Test-Path -LiteralPath $startMenuFolder -PathType Container) {
        Remove-Item -LiteralPath $startMenuFolder -ErrorAction SilentlyContinue
    }
}

# 重试删除系统临时目录下的单个测试根，容忍 WebView2 退出时异步释放或移走缓存文件。
function Remove-DshInstallerTestDirectory {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [int]$TimeoutSeconds = 30
    )
    $systemTempPath = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\')
    $resolved = [System.IO.Path]::GetFullPath($Root).TrimEnd('\')
    if (-not ([System.IO.Path]::GetDirectoryName($resolved)).Equals(
            $systemTempPath,
            [System.StringComparison]::OrdinalIgnoreCase
        ) -or
        -not ([System.IO.Path]::GetFileName($resolved)).StartsWith(
            $script:DshInstallerTestRootPrefix,
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
        throw "Refusing to delete an unexpected installer test root: $resolved"
    }

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        try {
            Remove-Item -LiteralPath $resolved -Recurse -Force -ErrorAction Stop
        } catch {
            if (-not (Test-Path -LiteralPath $resolved)) { return }
            if ((Get-Date) -ge $deadline) { throw }
            Start-Sleep -Milliseconds 250
        }
    } while (Test-Path -LiteralPath $resolved)
}
