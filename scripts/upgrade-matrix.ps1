param(
    [Parameter(Mandatory = $true)][string]$LegacyInstaller,
    [Parameter(Mandatory = $true)][string]$PayloadInstaller,
    [string]$PreviousPayloadInstaller,
    [int]$TimeoutSeconds = 180,
    [string]$Output
)

$ErrorActionPreference = 'Stop'
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
. (Join-Path $PSScriptRoot 'release-installer-isolation.ps1')
. (Join-Path $PSScriptRoot 'release-source.ps1')
$sourceCommit = Get-DshReleaseSourceCommit -RepoRoot $repoRoot
$package = Get-Content -LiteralPath (Join-Path $repoRoot 'package.json') -Raw | ConvertFrom-Json
$runtimeLock = Get-Content -LiteralPath (Join-Path $repoRoot 'runtime.lock.json') -Raw | ConvertFrom-Json
$defaultOutput = Join-Path $repoRoot ".release-work\$($package.version)\reports\upgrade-matrix.json"
$reportPath = if ([string]::IsNullOrWhiteSpace($Output)) {
    $defaultOutput
} elseif ([System.IO.Path]::IsPathRooted($Output)) {
    [System.IO.Path]::GetFullPath($Output)
} else {
    [System.IO.Path]::GetFullPath((Join-Path $repoRoot $Output))
}
$releaseRoot = [System.IO.Path]::GetFullPath((Join-Path $repoRoot ".release-work\$($package.version)"))
if (-not $reportPath.StartsWith($releaseRoot.TrimEnd('\') + '\', [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Upgrade matrix report must stay under $releaseRoot"
}

# 将仓库相对路径解析为已存在的安装器。
function Resolve-InstallerPath {
    param([Parameter(Mandatory = $true)][string]$Path)
    $resolved = if ([System.IO.Path]::IsPathRooted($Path)) {
        [System.IO.Path]::GetFullPath($Path)
    } else {
        [System.IO.Path]::GetFullPath((Join-Path $repoRoot $Path))
    }
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) { throw "Installer not found: $resolved" }
    return $resolved
}

# 创建一个直接位于系统临时目录的完整隔离安装上下文。
function New-MatrixContext {
    param([Parameter(Mandatory = $true)][string]$Scenario)
    $root = New-DshInstallerTestRoot
    $installRoot = Join-Path $root 'install'
    Assert-DshInstallerTestRoots -OwnedInstallRoots @($installRoot)
    $dshHome = Join-Path $root '.dsh'
    $localAppData = Join-Path $root 'localappdata'
    $webviewData = Join-Path $root 'webview-data'
    New-Item -ItemType Directory -Force -Path $dshHome, $localAppData, (Join-Path $root 'workspace'), $webviewData | Out-Null
    $sentinel = Join-Path $dshHome 'upgrade-matrix-sentinel.txt'
    Set-Content -LiteralPath $sentinel -Value 'must survive uninstall' -Encoding utf8NoBOM
    return [pscustomobject]@{
        Scenario = $Scenario
        Root = $root
        InstallRoot = $installRoot
        Exe = Join-Path $installRoot 'dsh-desktop.exe'
        Uninstaller = Join-Path $installRoot 'uninstall.exe'
        DshHome = $dshHome
        LocalAppData = $localAppData
        RuntimeRoot = Join-Path $localAppData 'dsh-desktop\runtime'
        Workspace = Join-Path $root 'workspace'
        WebViewData = $webviewData
        Sentinel = $sentinel
    }
}

# 设置当前场景的隔离用户目录和启动超时。
function Set-MatrixEnvironment {
    param([Parameter(Mandatory = $true)]$Context)
    $env:DSH_HOME = $Context.DshHome
    $env:LOCALAPPDATA = $Context.LocalAppData
    $env:DSH_DESKTOP_CWD = $Context.Workspace
    $env:DSH_DESKTOP_USER_HOME = $Context.Root
    $env:DSH_DESKTOP_READY_TIMEOUT_SECS = $TimeoutSeconds.ToString()
    $env:DSH_DESKTOP_CORE_READY_TIMEOUT_SECS = $TimeoutSeconds.ToString()
    $env:DSH_DESKTOP_PLUGIN_READY_TIMEOUT_SECS = $TimeoutSeconds.ToString()
    # 安装目录本身已隔离；默认 WebView2 数据目录会随临时安装目录一起清理。
    Remove-Item Env:DSH_DESKTOP_WEBVIEW_TEST_DATA_DIR -ErrorAction SilentlyContinue
}

# 只返回当前隔离安装目录拥有的桌面进程。
function Get-OwnedDesktopProcesses {
    param([Parameter(Mandatory = $true)]$Context)
    if (-not (Test-Path -LiteralPath $Context.Exe -PathType Leaf)) { return @() }
    $expected = [System.IO.Path]::GetFullPath($Context.Exe)
    return @(Get-Process dsh-desktop -ErrorAction SilentlyContinue | Where-Object {
        $_.Path -and [System.IO.Path]::GetFullPath($_.Path).Equals(
            $expected,
            [System.StringComparison]::OrdinalIgnoreCase
        )
    })
}

# 静默安装 legacy 或 payload，并等待关键资源与 provision 状态落盘。
function Install-MatrixBuild {
    param(
        [Parameter(Mandatory = $true)]$Context,
        [Parameter(Mandatory = $true)][string]$Installer,
        [Parameter(Mandatory = $true)][bool]$Payload
    )
    Set-MatrixEnvironment $Context
    $arguments = @('/S', '/NS')
    if ($Payload) { $arguments += "/PAYLOADTESTROOT=$($Context.RuntimeRoot)" }
    else { $arguments += "/DSHHOME=$($Context.DshHome)" }
    $arguments += "/D=$($Context.InstallRoot)"
    $installTimeoutSeconds = if ($Payload) { $TimeoutSeconds } else { [Math]::Max($TimeoutSeconds, 900) }
    $process = Start-Process -FilePath $Installer -ArgumentList $arguments -PassThru
    if (-not $process.WaitForExit($installTimeoutSeconds * 1000)) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        throw "Installer timed out after $installTimeoutSeconds seconds: $Installer"
    }
    if ($process.ExitCode -ne 0) { throw "Installer failed with code $($process.ExitCode): $Installer" }
    $deadline = (Get-Date).AddSeconds($installTimeoutSeconds)
    do {
        $ready = (Test-Path -LiteralPath $Context.Exe -PathType Leaf) -and
            (Test-Path -LiteralPath $Context.Uninstaller -PathType Leaf)
        if ($Payload) {
            $ready = $ready -and
                (Test-Path -LiteralPath (Join-Path $Context.InstallRoot 'payload-manifest.json') -PathType Leaf) -and
                (Test-Path -LiteralPath (Join-Path $Context.RuntimeRoot 'runtime-state.json') -PathType Leaf)
        } else {
            $ready = $ready -and (Test-Path -LiteralPath (Join-Path $Context.InstallRoot 'node\node.exe') -PathType Leaf)
        }
        if (-not $ready) { Start-Sleep -Milliseconds 250 }
    } while (-not $ready -and (Get-Date) -lt $deadline)
    if (-not $ready) { throw "Installed resources did not become ready: $($Context.InstallRoot)" }
}

# 读取并校验单一 runtime 状态文件。
function Get-RuntimeState {
    param([Parameter(Mandatory = $true)]$Context)
    $path = Join-Path $Context.RuntimeRoot 'runtime-state.json'
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Runtime state is missing: $path" }
    return Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
}

# 读取隔离 DSH_HOME 中的 web profile，供设置插件升级场景断言依赖与 bundle。
function Get-MatrixWebProfile {
    param([Parameter(Mandatory = $true)]$Context)
    $path = Join-Path $Context.DshHome 'profiles\web\package.json'
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Web profile is missing: $path" }
    return Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
}

# 写回测试专用 profile；只用于模拟用户在 preview.10 中卸载旧设置包。
function Set-MatrixWebProfile {
    param(
        [Parameter(Mandatory = $true)]$Context,
        [Parameter(Mandatory = $true)]$Profile
    )
    $path = Join-Path $Context.DshHome 'profiles\web\package.json'
    $Profile | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $path -Encoding utf8NoBOM
}

# 验证设置包的依赖和 bundle 状态，防止只改 dependency 或只改 activation 的半迁移。
function Assert-MatrixSettingsState {
    param(
        [Parameter(Mandatory = $true)]$Context,
        [Parameter(Mandatory = $true)][bool]$ExpectedNewSettings
    )
    $profile = Get-MatrixWebProfile $Context
    $hasLegacyDependency = $null -ne $profile.dependencies.PSObject.Properties['@dsh-desktop/theme-settings']
    $hasNewDependency = $null -ne $profile.dependencies.PSObject.Properties['@dsh-desktop/settings']
    $bundles = @($profile.dsh.profile.bundles)
    if ($hasLegacyDependency -or '@dsh-desktop/theme-settings' -in $bundles) {
        throw 'Legacy theme-settings remained after preview.11 coordination.'
    }
    if ($hasNewDependency -ne $ExpectedNewSettings -or
        (('@dsh-desktop/settings' -in $bundles) -ne $ExpectedNewSettings)) {
        throw "New settings state mismatch: expected=$ExpectedNewSettings dependency=$hasNewDependency"
    }
}

# 运行完整桌面 smoke，使 candidate 通过真实 Host/plugins readiness 后晋升。
function Invoke-MatrixSmoke {
    param(
        [Parameter(Mandatory = $true)]$Context,
        [switch]$AllowCandidateFallbackError
    )
    Set-MatrixEnvironment $Context
    $arguments = @{
        Exe = $Context.Exe
        TimeoutSeconds = $TimeoutSeconds
        UseBundledRuntime = $true
        UseInstalledWebViewDataDirectory = $true
        DshHome = $Context.DshHome
    }
    if ($AllowCandidateFallbackError) { $arguments.AllowCandidateFallbackError = $true }
    # smoke 失败使用 terminating error；成功后 LASTEXITCODE 可能仍保留其内部最后一个子进程状态。
    & (Join-Path $PSScriptRoot 'smoke-test.ps1') @arguments
}

# 启动一个保持运行的旧版本实例，供 NSIS 验证 --quit-existing 升级路径。
function Start-RunningDesktop {
    param(
        [Parameter(Mandatory = $true)]$Context,
        [switch]$AllowLegacyReadyLog
    )
    Set-MatrixEnvironment $Context
    $logDirectory = Join-Path $Context.Root 'running-upgrade-log'
    New-Item -ItemType Directory -Force -Path $logDirectory | Out-Null
    $env:DSH_DESKTOP_LOG_DIR = $logDirectory
    $logPath = Join-Path $logDirectory 'dsh-desktop.log'
    $process = Start-Process -FilePath $Context.Exe -PassThru
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $hostId = $null
    do {
        Start-Sleep -Milliseconds 100
        $process.Refresh()
        if ($process.HasExited) { throw "Old desktop exited before running-upgrade readiness: $($process.ExitCode)" }
        if (Test-Path -LiteralPath $logPath -PathType Leaf) {
            $content = Get-Content -LiteralPath $logPath -Raw
            $hostMatch = [regex]::Match($content, 'host started: pid=(\d+)')
            if ($hostMatch.Success) { $hostId = [int]$hostMatch.Groups[1].Value }
            $readyDuration = Get-DshDesktopReadyDuration -Content $content -AllowLegacyFormat:$AllowLegacyReadyLog
            $ready = $null -ne $readyDuration
        }
    } while (-not $ready -and (Get-Date) -lt $deadline)
    if (-not $ready -or $null -eq $hostId) { throw 'Old desktop did not reach CoreReady for running upgrade.' }
    return [pscustomobject]@{
        Process = $process
        HostId = $hostId
        LogPath = $logPath
        ReadyDurationMs = $readyDuration
    }
}

# 验证运行中升级确实通过旧版本退出链路收敛桌面和 Host。
function Assert-RunningDesktopStopped {
    param([Parameter(Mandatory = $true)]$Running)
    if (-not $Running.Process.WaitForExit(15000)) {
        throw 'Running upgrade did not stop the old desktop within 15 seconds.'
    }
    $deadline = (Get-Date).AddSeconds(10)
    while ((Get-Process -Id $Running.HostId -ErrorAction SilentlyContinue) -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 100
    }
    if (Get-Process -Id $Running.HostId -ErrorAction SilentlyContinue) {
        throw "Running upgrade left old Host PID $($Running.HostId) alive."
    }
}

# 调用安装版内置 provision helper，并返回退出码。
function Invoke-TestProvision {
    param(
        [Parameter(Mandatory = $true)]$Context,
        [Parameter(Mandatory = $true)][string]$Resources,
        [string]$RuntimeRoot = $Context.RuntimeRoot
    )
    Set-MatrixEnvironment $Context
    $captureId = [guid]::NewGuid().ToString('N').Substring(0, 12)
    $stdout = Join-Path $Context.Root "provision-$captureId.stdout.log"
    $stderr = Join-Path $Context.Root "provision-$captureId.stderr.log"
    $process = Start-Process -FilePath $Context.Exe -ArgumentList @(
        '--provision-runtime', '--provision-test-mode', '--payload-resources', $Resources,
        '--runtime-root', $RuntimeRoot
    ) -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
    if (-not $process.WaitForExit([Math]::Max($TimeoutSeconds, 900) * 1000)) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        throw "Provision helper timed out: resources=$Resources"
    }
    if ($process.ExitCode -ne 0 -and (Test-Path -LiteralPath $stderr -PathType Leaf)) {
        $message = (Get-Content -LiteralPath $stderr -Raw).Trim()
        if (-not [string]::IsNullOrWhiteSpace($message)) {
            Write-Host "PROVISION REJECTED: $message"
        }
    }
    return $process.ExitCode
}

# 复制安装器四个 payload 资源，供损坏输入与 candidate fixture 使用。
function Copy-PayloadResources {
    param(
        [Parameter(Mandatory = $true)]$Context,
        [Parameter(Mandatory = $true)][string]$Destination
    )
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    foreach ($name in @('payload-manifest.json', 'node-runtime.zip', 'host-runtime.zip', 'builtin-plugins.zip')) {
        Copy-Item -LiteralPath (Join-Path $Context.InstallRoot $name) -Destination (Join-Path $Destination $name) -Force
    }
}

# 生成摘要合法但 Host 入口立即退出的 candidate payload。
function New-ReadinessFailurePayload {
    param(
        [Parameter(Mandatory = $true)]$Context,
        [Parameter(Mandatory = $true)][string]$Destination
    )
    $state = Get-RuntimeState $Context
    if ($null -eq $state.active) { throw 'Readiness failure fixture requires an active runtime.' }
    $activeRoot = Join-Path $Context.RuntimeRoot $state.active.payloadDigest
    $source = Join-Path $Context.Root 'readiness-failure-source'
    New-Item -ItemType Directory -Force -Path $source | Out-Null
    foreach ($name in @('node', 'host', 'plugins')) {
        $wrapper = Join-Path $source $name
        New-Item -ItemType Directory -Force -Path $wrapper | Out-Null
        Copy-Item -LiteralPath (Join-Path $activeRoot $name) -Destination $wrapper -Recurse -Force
    }
    $cli = Join-Path $source 'host\host\node_modules\@deepseek-ai\dsh\lib\bin.js'
    "process.stderr.write('intentional candidate readiness failure\n'); process.exit(97);`n" |
        Set-Content -LiteralPath $cli -Encoding utf8NoBOM
    & cargo run --quiet --manifest-path (Join-Path $repoRoot 'src-tauri\Cargo.toml') --example payload-tool -- package `
        --node (Join-Path $source 'node') `
        --host (Join-Path $source 'host') `
        --plugins (Join-Path $source 'plugins') `
        --output $Destination `
        --desktop-version "$($package.version)-readiness-failure" `
        --node-version $runtimeLock.node.version `
        --pnpm-version $runtimeLock.pnpm.version `
        --runtime-abi 1
    if ($LASTEXITCODE -ne 0) { throw 'Could not build readiness failure payload.' }
}

# 静默卸载并确认托管 runtime 删除、用户 profile sentinel 保留。
function Uninstall-And-Verify {
    param([Parameter(Mandatory = $true)]$Context)
    if (-not (Test-Path -LiteralPath $Context.Uninstaller -PathType Leaf)) { throw 'Uninstaller is missing.' }
    Set-MatrixEnvironment $Context
    Invoke-DshSilentUninstall `
        -Uninstaller $Context.Uninstaller `
        -CompletionPaths @($Context.Exe, $Context.RuntimeRoot)
    if (-not (Test-Path -LiteralPath $Context.Sentinel -PathType Leaf)) { throw 'Uninstaller removed profile sentinel.' }
}

# 删除已成功场景的临时根；失败场景始终保留诊断。
function Remove-SuccessfulContext {
    param([Parameter(Mandatory = $true)]$Context)
    Remove-DshInstallerTestDirectory -Root $Context.Root
}

$legacyPath = Resolve-InstallerPath $LegacyInstaller
$payloadPath = Resolve-InstallerPath $PayloadInstaller
$previousPayloadPath = if ([string]::IsNullOrWhiteSpace($PreviousPayloadInstaller)) { $null } else { Resolve-InstallerPath $PreviousPayloadInstaller }
$baselineHash = (Get-FileHash -LiteralPath $legacyPath -Algorithm SHA256).Hash.ToLowerInvariant()
$expectedBaselineHash = 'e331e628b07bf574e823610324130c258d77ed1e57113b59426feed1a3a9d3d9'
if ($baselineHash -ne $expectedBaselineHash) { throw "preview.7 baseline SHA-256 mismatch: $baselineHash" }
$payloadHash = (Get-FileHash -LiteralPath $payloadPath -Algorithm SHA256).Hash.ToLowerInvariant()
Assert-DshInstallerTestUserIsClean
$scenarioResults = @()
$retainedRoots = @()
$ownedInstallRoots = @()
$environmentNames = @(
    'DSH_HOME', 'LOCALAPPDATA', 'DSH_DESKTOP_CWD', 'DSH_DESKTOP_LOG_DIR', 'DSH_DESKTOP_USER_HOME',
    'DSH_DESKTOP_READY_TIMEOUT_SECS', 'DSH_DESKTOP_CORE_READY_TIMEOUT_SECS', 'DSH_DESKTOP_PLUGIN_READY_TIMEOUT_SECS',
    'DSH_DESKTOP_WEBVIEW_TEST_DATA_DIR'
)
$previousEnvironment = @{}
foreach ($name in $environmentNames) { $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }

# 执行一个独立矩阵场景；成功后验证卸载并回收，失败则记录诊断根。
function Invoke-MatrixScenario {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][scriptblock]$Body
    )
    $context = New-MatrixContext $Name
    $script:ownedInstallRoots += $context.InstallRoot
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        & $Body $context
        foreach ($process in @(Get-OwnedDesktopProcesses $context)) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        }
        Uninstall-And-Verify $context
        Remove-SuccessfulContext $context
        $watch.Stop()
        $script:scenarioResults += [pscustomobject]@{ name = $Name; passed = $true; durationMs = $watch.ElapsedMilliseconds }
    }
    catch {
        $watch.Stop()
        foreach ($process in @(Get-OwnedDesktopProcesses $context)) {
            Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        }
        $script:retainedRoots += $context.Root
        $script:scenarioResults += [pscustomobject]@{
            name = $Name
            passed = $false
            durationMs = $watch.ElapsedMilliseconds
            error = $_.Exception.Message
            diagnostics = $context.Root
        }
        throw
    }
}

$matrixPassed = $false
$failure = $null
try {
    Invoke-MatrixScenario 'payload-clean' {
        param($context)
        Install-MatrixBuild $context $payloadPath $true
        $state = Get-RuntimeState $context
        if ($null -eq $state.candidate -or $null -ne $state.active) { throw 'Clean payload install did not register exactly one candidate.' }
        Invoke-MatrixSmoke $context
        $state = Get-RuntimeState $context
        if ($null -eq $state.active -or $null -ne $state.candidate) { throw 'Clean candidate was not promoted.' }
    }

    Invoke-MatrixScenario 'legacy-stopped-upgrade' {
        param($context)
        Install-MatrixBuild $context $legacyPath $false
        Install-MatrixBuild $context $payloadPath $true
        $state = Get-RuntimeState $context
        if ($null -eq $state.candidate -or $null -ne $state.active) { throw 'Legacy stopped upgrade did not preserve candidate semantics.' }
        Invoke-MatrixSmoke $context
    }

    Invoke-MatrixScenario 'legacy-running-upgrade' {
        param($context)
        Install-MatrixBuild $context $legacyPath $false
        $running = Start-RunningDesktop $context -AllowLegacyReadyLog
        Install-MatrixBuild $context $payloadPath $true
        Assert-RunningDesktopStopped $running
        Invoke-MatrixSmoke $context
    }

    Invoke-MatrixScenario 'payload-corruption-and-candidate-rollback' {
        param($context)
        Install-MatrixBuild $context $payloadPath $true
        Invoke-MatrixSmoke $context
        $activeStatePath = Join-Path $context.RuntimeRoot 'runtime-state.json'
        $originalStateBytes = [System.IO.File]::ReadAllBytes($activeStatePath)

        $invalidManifest = Join-Path $context.Root 'invalid-manifest'
        Copy-PayloadResources $context $invalidManifest
        Set-Content -LiteralPath (Join-Path $invalidManifest 'payload-manifest.json') -Value '{' -Encoding ascii
        $cleanFailureRoot = Join-Path $context.Root 'clean-failure-runtime'
        if ((Invoke-TestProvision $context $invalidManifest $cleanFailureRoot) -eq 0) { throw 'Invalid manifest unexpectedly provisioned.' }
        if (Test-Path -LiteralPath (Join-Path $cleanFailureRoot 'runtime-state.json')) { throw 'Clean provision failure wrote runtime state.' }

        $truncated = Join-Path $context.Root 'truncated-zip'
        Copy-PayloadResources $context $truncated
        $zipPath = Join-Path $truncated 'host-runtime.zip'
        $stream = [System.IO.File]::Open($zipPath, 'Open', 'Write', 'None')
        try { $stream.SetLength([Math]::Max(1, [Math]::Floor($stream.Length / 2))) } finally { $stream.Dispose() }
        if ((Invoke-TestProvision $context $truncated) -eq 0) { throw 'Truncated ZIP unexpectedly provisioned.' }
        $restoredStateBytes = [System.IO.File]::ReadAllBytes($activeStatePath)
        if ([Convert]::ToHexString($originalStateBytes) -ne [Convert]::ToHexString($restoredStateBytes)) {
            throw 'Failed upgrade provision changed active runtime state bytes.'
        }

        $candidateResources = Join-Path $context.Root 'readiness-failure-payload'
        New-ReadinessFailurePayload $context $candidateResources
        if ((Invoke-TestProvision $context $candidateResources) -ne 0) { throw 'Valid readiness failure candidate could not be provisioned.' }
        $candidateState = Get-RuntimeState $context
        $originalActive = $candidateState.active.payloadDigest
        $failedDigest = $candidateState.candidate.payloadDigest
        if ($null -eq $failedDigest -or $failedDigest -eq $originalActive) { throw 'Readiness fixture did not create a distinct candidate.' }
        Invoke-MatrixSmoke $context -AllowCandidateFallbackError
        $rolledBack = Get-RuntimeState $context
        if ($rolledBack.active.payloadDigest -ne $originalActive -or $null -ne $rolledBack.candidate) {
            throw 'Readiness failure did not reject candidate and continue the old active runtime.'
        }
        if ((Invoke-TestProvision $context $context.InstallRoot) -ne 0) { throw 'Could not reprovision active payload for garbage collection.' }
        if (Test-Path -LiteralPath (Join-Path $context.RuntimeRoot $failedDigest)) { throw 'Garbage collection retained unreferenced failed candidate.' }
    }

    if ($null -ne $previousPayloadPath) {
        foreach ($mode in @('stopped', 'running')) {
            Invoke-MatrixScenario "payload-$mode-upgrade" {
                param($context)
                Install-MatrixBuild $context $previousPayloadPath $true
                Invoke-MatrixSmoke $context
                $oldState = Get-RuntimeState $context
                $oldDigest = $oldState.active.payloadDigest
                $running = if ($mode -eq 'running') { Start-RunningDesktop $context } else { $null }
                Install-MatrixBuild $context $payloadPath $true
                if ($null -ne $running) { Assert-RunningDesktopStopped $running }
                $candidate = Get-RuntimeState $context
                if ($candidate.candidate.payloadDigest -eq $oldDigest) { throw 'Payload upgrade did not produce a new candidate digest.' }
                Invoke-MatrixSmoke $context
                $promoted = Get-RuntimeState $context
                if ($promoted.previous.payloadDigest -ne $oldDigest -or $null -ne $promoted.candidate) {
                    throw 'Payload upgrade did not retain previous or clear candidate.'
                }
            }
        }

        Invoke-MatrixScenario 'payload-settings-migration' {
            param($context)
            Install-MatrixBuild $context $previousPayloadPath $true
            Invoke-MatrixSmoke $context
            $oldProfile = Get-MatrixWebProfile $context
            if ($null -eq $oldProfile.dependencies.PSObject.Properties['@dsh-desktop/theme-settings']) {
                throw 'Preview.10 fixture does not contain managed theme-settings.'
            }
            $hindsightPath = Join-Path $context.Root '.hindsight\coding-agent.json'
            New-Item -ItemType Directory -Force -Path (Split-Path $hindsightPath -Parent) | Out-Null
            $hindsightBytes = [Text.Encoding]::UTF8.GetBytes('{"apiUrl":"http://127.0.0.1:8888","custom":"preserve"}')
            [IO.File]::WriteAllBytes($hindsightPath, $hindsightBytes)

            Install-MatrixBuild $context $payloadPath $true
            Invoke-MatrixSmoke $context
            Assert-MatrixSettingsState $context $true
            if ([Convert]::ToHexString([IO.File]::ReadAllBytes($hindsightPath)) -ne
                [Convert]::ToHexString($hindsightBytes)) {
                throw 'Settings migration changed existing Hindsight configuration bytes.'
            }
        }

        Invoke-MatrixScenario 'payload-settings-uninstalled-upgrade' {
            param($context)
            Install-MatrixBuild $context $previousPayloadPath $true
            Invoke-MatrixSmoke $context
            $profile = Get-MatrixWebProfile $context
            $profile.dependencies.PSObject.Properties.Remove('@dsh-desktop/theme-settings')
            $profile.dsh.profile.bundles = @(
                $profile.dsh.profile.bundles | Where-Object { $_ -ne '@dsh-desktop/theme-settings' }
            )
            Set-MatrixWebProfile $context $profile

            Install-MatrixBuild $context $payloadPath $true
            Invoke-MatrixSmoke $context
            Assert-MatrixSettingsState $context $false
            Invoke-MatrixSmoke $context
            Assert-MatrixSettingsState $context $false
        }
    }
    $matrixPassed = $true
}
catch {
    $failure = $_.Exception.Message
}
finally {
    try {
        Clear-DshInstallerTestUserState -OwnedInstallRoots $ownedInstallRoots
    } catch {
        if ($null -eq $failure) { $failure = $_.Exception.Message }
        $matrixPassed = $false
    }
    foreach ($name in $previousEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], 'Process')
    }
    $report = [ordered]@{
        schemaVersion = 2
        generatedAtUtc = [DateTime]::UtcNow.ToString('O')
        desktopVersion = $package.version
        sourceCommit = $sourceCommit
        installers = [ordered]@{
            legacy = [ordered]@{ version = '0.1.0-preview.7'; path = $legacyPath; sha256 = $baselineHash }
            payload = [ordered]@{ version = $package.version; path = $payloadPath; sha256 = $payloadHash }
            previousPayload = if ($null -eq $previousPayloadPath) { $null } else { [ordered]@{
                path = $previousPayloadPath
                sha256 = (Get-FileHash -LiteralPath $previousPayloadPath -Algorithm SHA256).Hash.ToLowerInvariant()
            } }
        }
        isolated = [ordered]@{ localAppData = $true; dshHome = $true; installDirectory = $true }
        scenarios = $scenarioResults
        retainedDiagnostics = $retainedRoots
        passed = $matrixPassed
        failure = $failure
    }
    New-Item -ItemType Directory -Force -Path (Split-Path $reportPath -Parent) | Out-Null
    $report | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $reportPath -Encoding utf8NoBOM
}

if (-not $matrixPassed) { throw "Upgrade matrix failed: $failure; report=$reportPath" }
Write-Host "UPGRADE MATRIX OK: $($scenarioResults.Count) scenarios, report=$reportPath"
