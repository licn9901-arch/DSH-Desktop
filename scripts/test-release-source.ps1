$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'release-source.ps1')

$root = Join-Path ([System.IO.Path]::GetTempPath()) "dsh-release-source-$PID-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $root | Out-Null
try {
    & git.exe -C $root init --quiet
    if ($LASTEXITCODE -ne 0) { throw 'Could not initialize release source fixture.' }
    Set-Content -LiteralPath (Join-Path $root 'tracked.txt') -Value 'tracked' -Encoding utf8NoBOM
    & git.exe -C $root add -- tracked.txt
    & git.exe -C $root -c user.name='DSH Release Test' -c user.email='release-test@example.invalid' commit --quiet -m 'test fixture'
    if ($LASTEXITCODE -ne 0) { throw 'Could not commit release source fixture.' }

    $commit = Get-DshReleaseSourceCommit -RepoRoot $root
    if ($commit -notmatch '^[0-9a-f]{40}$') { throw "Unexpected commit: $commit" }
    Assert-DshReleaseWorktreeClean -RepoRoot $root

    Set-Content -LiteralPath (Join-Path $root 'untracked.txt') -Value 'dirty' -Encoding utf8NoBOM
    $rejected = $false
    try { Assert-DshReleaseWorktreeClean -RepoRoot $root } catch { $rejected = $true }
    if (-not $rejected) { throw 'Dirty release fixture was not rejected.' }
}
finally {
    if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
}

Write-Host 'RELEASE SOURCE TEST OK: clean commit accepted and dirty worktree rejected.'
