$ErrorActionPreference = 'Stop'

# 解析桌面 preview 版本，并拒绝 payload 灰度开始前或其他版本线的输入。
function Get-DshPreviewNumber {
    param([Parameter(Mandatory = $true)][string]$Version)
    $match = [regex]::Match($Version, '^0\.1\.0-preview\.(\d+)$')
    if (-not $match.Success) {
        throw "Release gate requires a 0.1.0 preview version, got $Version."
    }
    $number = [int]$match.Groups[1].Value
    if ($number -lt 8) {
        throw "Release gate only accepts preview.8 or newer, got $Version."
    }
    return $number
}

# 校验直接前序安装器；完整门禁报告优先，公开热修版可使用同名 SHA-256 文件作为证据。
function Get-DshPreviousInstallerEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$InstallerPath,
        [Parameter(Mandatory = $true)][string]$ExpectedVersion,
        [string]$GateReportPath,
        [string]$AssetDigest
    )
    $installer = [System.IO.Path]::GetFullPath($InstallerPath)
    if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) {
        throw "Previous payload installer is missing: $installer"
    }
    $expectedName = "DeepSeek.Harness.Desktop_$($ExpectedVersion)_x64-setup.exe"
    if ([System.IO.Path]::GetFileName($installer) -ne $expectedName) {
        throw "Previous payload installer name mismatch: expected $expectedName."
    }
    $actualHash = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()

    if (-not [string]::IsNullOrWhiteSpace($GateReportPath) -and
        (Test-Path -LiteralPath $GateReportPath -PathType Leaf)) {
        $report = Get-Content -LiteralPath $GateReportPath -Raw | ConvertFrom-Json
        if (-not $report.passed -or $report.desktopVersion -ne $ExpectedVersion) {
            throw "Previous payload gate report is not a passing $ExpectedVersion report."
        }
        $expectedHash = [string]$report.installers.payload.sha256
        if ($actualHash -ne $expectedHash.ToLowerInvariant()) {
            throw "Previous payload installer hash does not match its gate report."
        }
        return [pscustomobject]@{ Sha256 = $actualHash; Source = 'release-gate'; Path = $GateReportPath }
    }

    if ([string]::IsNullOrWhiteSpace($AssetDigest)) {
        throw "GitHub Asset digest is required when the previous preview has no gate report."
    }
    $assetMatch = [regex]::Match($AssetDigest.Trim(), '^sha256:([0-9a-fA-F]{64})$')
    if (-not $assetMatch.Success) {
        throw "GitHub Asset digest format is invalid: $AssetDigest"
    }
    $assetHash = $assetMatch.Groups[1].Value.ToLowerInvariant()
    if ($actualHash -ne $assetHash) {
        throw "Previous payload installer hash does not match its GitHub Asset digest."
    }

    $checksumPath = "$installer.sha256"
    if (-not (Test-Path -LiteralPath $checksumPath -PathType Leaf)) {
        throw "Previous payload evidence is missing: $checksumPath"
    }
    $checksum = (Get-Content -LiteralPath $checksumPath -Raw).Trim()
    $escapedName = [regex]::Escape($expectedName)
    $checksumMatch = [regex]::Match($checksum, "^([0-9a-fA-F]{64})\s+\*?$escapedName$")
    if (-not $checksumMatch.Success) {
        throw "Previous payload checksum format is invalid: $checksumPath"
    }
    $expectedHash = $checksumMatch.Groups[1].Value.ToLowerInvariant()
    if ($actualHash -ne $expectedHash) {
        throw "Previous payload installer hash does not match $checksumPath."
    }
    return [pscustomobject]@{
        Sha256 = $actualHash
        Source = 'sha256+github-asset'
        Path = $checksumPath
        AssetDigest = "sha256:$assetHash"
    }
}
