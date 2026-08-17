param(
    [string]$ResourceRoot
)

$arguments = @()
if ($ResourceRoot) { $arguments += @('--resource-root', $ResourceRoot) }
& node (Join-Path $PSScriptRoot 'verify-plugins.mjs') @arguments
exit $LASTEXITCODE
