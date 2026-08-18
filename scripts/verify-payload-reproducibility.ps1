param([string]$Output)

$ErrorActionPreference = 'Stop'
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$package = Get-Content -LiteralPath (Join-Path $repoRoot 'package.json') -Raw | ConvertFrom-Json
$releaseRoot = [System.IO.Path]::GetFullPath((Join-Path $repoRoot ".release-work\$($package.version)"))
$reportPath = if ([string]::IsNullOrWhiteSpace($Output)) {
    Join-Path $releaseRoot 'reports\payload-reproducibility.json'
} elseif ([System.IO.Path]::IsPathRooted($Output)) {
    [System.IO.Path]::GetFullPath($Output)
} else {
    [System.IO.Path]::GetFullPath((Join-Path $repoRoot $Output))
}
if (-not $reportPath.StartsWith($releaseRoot.TrimEnd('\') + '\', [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Reproducibility report must stay under $releaseRoot"
}
$resources = Join-Path $repoRoot 'src-tauri\resources\payload'
$names = @('payload-manifest.json', 'node-runtime.zip', 'host-runtime.zip', 'builtin-plugins.zip')

# 强制生成一轮 payload，并捕获四个发布资源的稳定摘要和大小。
function Invoke-ReproducibleBuild {
    param([Parameter(Mandatory = $true)][int]$Run)
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    & (Join-Path $PSScriptRoot 'stage-payload.ps1') -Force | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "Forced payload build $Run failed." }
    $watch.Stop()
    $files = [ordered]@{}
    foreach ($name in $names) {
        $path = Join-Path $resources $name
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Payload output is missing after run ${Run}: $name" }
        $item = Get-Item -LiteralPath $path
        $files[$name] = [ordered]@{
            sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
            bytes = $item.Length
        }
    }
    return [ordered]@{ run = $Run; durationMs = $watch.ElapsedMilliseconds; files = $files }
}

$first = Invoke-ReproducibleBuild 1
$second = Invoke-ReproducibleBuild 2
$mismatches = @()
foreach ($name in $names) {
    if ($first.files[$name].sha256 -ne $second.files[$name].sha256 -or
        $first.files[$name].bytes -ne $second.files[$name].bytes) {
        $mismatches += $name
    }
}
$report = [ordered]@{
    schemaVersion = 1
    generatedAtUtc = [DateTime]::UtcNow.ToString('O')
    desktopVersion = $package.version
    compression = [ordered]@{ method = 'Deflate'; level = 6; sortedPaths = $true; fixedTimestamp = $true }
    runs = @($first, $second)
    mismatches = $mismatches
    passed = $mismatches.Count -eq 0
}
New-Item -ItemType Directory -Force -Path (Split-Path $reportPath -Parent) | Out-Null
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $reportPath -Encoding utf8NoBOM
if ($mismatches.Count -gt 0) { throw "Payload reproducibility mismatch: $($mismatches -join ', ')" }
Write-Host "PAYLOAD REPRODUCIBILITY OK: 4 resources match across two forced builds, report=$reportPath"
