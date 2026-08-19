$ErrorActionPreference = 'Stop'

# 获取当前发布仓库 HEAD，并拒绝非完整 Git 提交摘要。
function Get-DshReleaseSourceCommit {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)
    $safeDirectory = [System.IO.Path]::GetFullPath($RepoRoot).Replace('\', '/')
    $commit = (& git.exe -C $RepoRoot -c "safe.directory=$safeDirectory" rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $commit -notmatch '^[0-9a-f]{40}$') {
        throw "Could not resolve a full release source commit: $commit"
    }
    return $commit
}

# 要求 tracked、staged 和 untracked 文件全部干净，避免从未提交源码生成正式制品。
function Assert-DshReleaseWorktreeClean {
    param([Parameter(Mandatory = $true)][string]$RepoRoot)
    $safeDirectory = [System.IO.Path]::GetFullPath($RepoRoot).Replace('\', '/')
    $changes = @(& git.exe -C $RepoRoot -c "safe.directory=$safeDirectory" status --porcelain=v1 --untracked-files=all)
    if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the release Git worktree.' }
    if ($changes.Count -gt 0) {
        throw "Release worktree is not clean: $($changes -join '; ')"
    }
}
