param(
    [string]$Output
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
. (Join-Path $PSScriptRoot 'release-source.ps1')
$sourceCommit = Get-DshReleaseSourceCommit -RepoRoot $repoRoot
$package = Get-Content -LiteralPath (Join-Path $repoRoot 'package.json') -Raw | ConvertFrom-Json
$releaseRoot = Join-Path $repoRoot ".release-work\$($package.version)"
$outputPath = if ([string]::IsNullOrWhiteSpace($Output)) {
    Join-Path $releaseRoot 'reports\npm-audit.json'
}
else {
    [System.IO.Path]::GetFullPath((Join-Path $repoRoot $Output))
}

# 审计报告只能写入当前版本的隔离发布工作目录。
$releasePrefix = [System.IO.Path]::GetFullPath($releaseRoot).TrimEnd('\') + '\'
if (-not $outputPath.StartsWith($releasePrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Audit output must stay under ${releaseRoot}: $outputPath"
}

# 执行一个 npm audit 并保留完整结构，调用方只根据稳定的漏洞计数做门禁判断。
function Invoke-ProjectAudit {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Directory,
        [switch]$ProductionOnly
    )
    $arguments = @(
        'audit', '--json', '--registry=https://registry.npmjs.org',
        '--cache', (Join-Path $repoRoot ".runtime-cache\npm-audit-$Name")
    )
    if ($ProductionOnly) { $arguments += '--omit=dev' }
    Push-Location $Directory
    try {
        $raw = & npm.cmd @arguments 2>$null
        $exitCode = $LASTEXITCODE
    }
    finally {
        Pop-Location
    }
    if ([string]::IsNullOrWhiteSpace(($raw -join "`n"))) {
        throw "npm audit returned no JSON for $Name (exit code $exitCode)."
    }
    try {
        $parsed = ($raw -join "`n") | ConvertFrom-Json
    }
    catch {
        throw "npm audit returned invalid JSON for ${Name}: $($_.Exception.Message)"
    }
    if ($null -eq $parsed.metadata.vulnerabilities) {
        throw "npm audit report for $Name has no vulnerability metadata."
    }
    [ordered]@{
        name = $Name
        directory = [System.IO.Path]::GetRelativePath($repoRoot, $Directory).Replace('\', '/')
        productionOnly = [bool]$ProductionOnly
        exitCode = $exitCode
        metadata = $parsed.metadata
        vulnerabilities = $parsed.vulnerabilities
    }
}

$reports = @(
    Invoke-ProjectAudit -Name 'root' -Directory $repoRoot
    Invoke-ProjectAudit -Name 'runtime-host' -Directory (Join-Path $repoRoot 'runtime-host') -ProductionOnly
    Invoke-ProjectAudit -Name 'plugin-runtime' -Directory (Join-Path $repoRoot 'plugin-runtime') -ProductionOnly
)
$result = [ordered]@{
    schemaVersion = 2
    generatedAtUtc = [DateTime]::UtcNow.ToString('O')
    desktopVersion = $package.version
    sourceCommit = $sourceCommit
    nodeVersion = (& node.exe --version).TrimStart('v')
    npmVersion = (& npm.cmd --version).Trim()
    projects = $reports
}
New-Item -ItemType Directory -Force -Path (Split-Path $outputPath -Parent) | Out-Null
$result | ConvertTo-Json -Depth 100 | Set-Content -LiteralPath $outputPath -Encoding utf8NoBOM

$failures = @($reports | Where-Object {
    [int]$_.metadata.vulnerabilities.total -ne 0 -or $_.exitCode -ne 0
})
if ($failures.Count -gt 0) {
    $summary = $failures | ForEach-Object {
        "$($_.name)=$($_.metadata.vulnerabilities | ConvertTo-Json -Compress)"
    }
    throw "Release audit failed: $($summary -join '; '). Report: $outputPath"
}
Write-Host "RELEASE AUDIT OK: 3 projects, 0 vulnerabilities, report=$outputPath"
