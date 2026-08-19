param(
    [Parameter(Mandatory = $true)][string]$HostRoot
)

$ErrorActionPreference = 'Stop'

$resolvedHostRoot = [System.IO.Path]::GetFullPath($HostRoot)
$workerPath = Join-Path $resolvedHostRoot 'node_modules\@deepseek-ai\dsh-host-directory-picker-native\lib\worker.cjs'
if (-not (Test-Path -LiteralPath $workerPath -PathType Leaf)) {
    throw "Native directory picker worker is missing: $workerPath"
}

$content = Get-Content -LiteralPath $workerPath -Raw
$replacements = @(
    @{
        Old = 'show: () => method(dialog, SLOT_SHOW, protoShow)(null),'
        New = 'show: (ownerHwnd) => method(dialog, SLOT_SHOW, protoShow)(ownerHwnd),'
    },
    @{
        Old = 'function runFolderDialog(bindings, title, onShowing) {'
        New = 'function runFolderDialog(bindings, title, ownerHwnd, onShowing) {'
    },
    @{
        Old = 'const shown = dialog.show();'
        New = 'const shown = dialog.show(ownerHwnd);'
    },
    @{
        Old = 'const title = process.env.DSH_DIALOG_TITLE ?? "";'
        New = @'
function ownerHwndFromEnvironment(value) {
	if (value === void 0 || value === "") return null;
	if (!/^[1-9][0-9]*$/.test(value)) throw new Error("win32-dialog-worker: DSH_DIRECTORY_PICKER_OWNER_HWND must be a positive decimal integer");
	const ownerHwnd = Number(value);
	if (!Number.isSafeInteger(ownerHwnd)) throw new Error("win32-dialog-worker: DSH_DIRECTORY_PICKER_OWNER_HWND exceeds the safe integer range");
	return ownerHwnd;
}
const title = process.env.DSH_DIALOG_TITLE ?? "";
const ownerHwnd = ownerHwndFromEnvironment(process.env.DSH_DIRECTORY_PICKER_OWNER_HWND);
'@
    },
    @{
        Old = 'path: runFolderDialog(await loadWin32DialogBindings(), title, (threadId) => {'
        New = 'path: runFolderDialog(await loadWin32DialogBindings(), title, ownerHwnd, (threadId) => {'
    }
)

foreach ($replacement in $replacements) {
    if ($content.Contains($replacement.New)) {
        continue
    }
    if (-not $content.Contains($replacement.Old)) {
        throw "Native directory picker source changed before the owner-window patch could be applied: $($replacement.Old)"
    }
    $content = $content.Replace($replacement.Old, $replacement.New)
}

Set-Content -LiteralPath $workerPath -Value $content -Encoding utf8NoBOM -NoNewline
Write-Host "Patched native directory picker owner window: $workerPath"
