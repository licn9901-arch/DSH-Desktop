param(
    [switch]$Offline
)

$arguments = @()
if ($Offline) { $arguments += '--offline' }
& node (Join-Path $PSScriptRoot 'stage-runtime.mjs') @arguments
exit $LASTEXITCODE
