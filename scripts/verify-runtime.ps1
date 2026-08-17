param(
    [string]$ResourceRoot,
    [string]$ArchivePath
)

$arguments = @()
if ($ResourceRoot) { $arguments += @('--resource-root', $ResourceRoot) }
if ($ArchivePath) { $arguments += @('--archive-path', $ArchivePath) }
& node (Join-Path $PSScriptRoot 'verify-runtime.mjs') @arguments
exit $LASTEXITCODE
