param(
    [switch]$Offline
)

$arguments = @()
if ($Offline) { $arguments += '--offline' }
& node (Join-Path $PSScriptRoot 'stage-plugins.mjs') @arguments
exit $LASTEXITCODE
