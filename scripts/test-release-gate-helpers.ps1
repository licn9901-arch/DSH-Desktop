$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'release-gate-helpers.ps1')

foreach ($case in @(
    @{ Version = '0.1.0-preview.8'; Number = 8 },
    @{ Version = '0.1.0-preview.13'; Number = 13 },
    @{ Version = '0.1.0-preview.120'; Number = 120 }
)) {
    $actual = Get-DshPreviewNumber -Version $case.Version
    if ($actual -ne $case.Number) {
        throw "Preview number mismatch for $($case.Version): $actual"
    }
}

foreach ($invalid in @('0.1.0-preview.7', '0.1.1-preview.13', '0.1.0-preview.x')) {
    $rejected = $false
    try { Get-DshPreviewNumber -Version $invalid | Out-Null } catch { $rejected = $true }
    if (-not $rejected) { throw "Invalid preview version was accepted: $invalid" }
}

$root = Join-Path ([System.IO.Path]::GetTempPath()) "dsh-release-gate-helper-$PID-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $root | Out-Null
try {
    $installer = Join-Path $root 'DeepSeek.Harness.Desktop_0.1.0-preview.12_x64-setup.exe'
    Set-Content -LiteralPath $installer -Value 'preview-12' -Encoding ascii
    $hash = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $([System.IO.Path]::GetFileName($installer))" |
        Set-Content -LiteralPath "$installer.sha256" -Encoding ascii

    $rejected = $false
    try {
        Get-DshPreviousInstallerEvidence -InstallerPath $installer -ExpectedVersion '0.1.0-preview.12' | Out-Null
    }
    catch { $rejected = $true }
    if (-not $rejected) { throw 'Checksum-only previous installer evidence was accepted.' }

    $resolved = Get-DshPreviousInstallerEvidence `
        -InstallerPath $installer `
        -ExpectedVersion '0.1.0-preview.12' `
        -AssetDigest "sha256:$hash"
    if ($resolved.Sha256 -ne $hash -or $resolved.Source -ne 'sha256+github-asset') {
        throw 'Published hotfix checksum evidence was not accepted.'
    }

    $gateReport = Join-Path $root 'release-gate.json'
    @{
        passed = $true
        desktopVersion = '0.1.0-preview.12'
        installers = @{ payload = @{ sha256 = $hash } }
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $gateReport -Encoding utf8NoBOM
    $resolved = Get-DshPreviousInstallerEvidence `
        -InstallerPath $installer `
        -ExpectedVersion '0.1.0-preview.12' `
        -GateReportPath $gateReport
    if ($resolved.Source -ne 'release-gate') {
        throw 'Passing release gate report was not preferred.'
    }

    $rejected = $false
    try {
        Get-DshPreviousInstallerEvidence `
            -InstallerPath $installer `
            -ExpectedVersion '0.1.0-preview.12' `
            -AssetDigest ('sha256:' + ('0' * 64)) | Out-Null
    }
    catch { $rejected = $true }
    if (-not $rejected) { throw 'Mismatched GitHub Asset digest was accepted.' }

    Set-Content -LiteralPath "$installer.sha256" -Value ('0' * 64) -Encoding ascii
    $rejected = $false
    try {
        Get-DshPreviousInstallerEvidence `
            -InstallerPath $installer `
            -ExpectedVersion '0.1.0-preview.12' `
            -AssetDigest "sha256:$hash" | Out-Null
    }
    catch { $rejected = $true }
    if (-not $rejected) { throw 'Mismatched previous installer checksum was accepted.' }
}
finally {
    if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
}

Write-Host 'RELEASE GATE HELPER TEST OK: preview parsing and three-way previous asset evidence verified.'
