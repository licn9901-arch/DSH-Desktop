# DeepSeek Harness Desktop

<p align="center">
  <a href="README.md">简体中文</a> · <strong>English</strong>
</p>

<p align="center">
  <img src="src-tauri/icons/icon.png" width="112" height="112" alt="DeepSeek Harness Desktop icon">
</p>

<p align="center">
  <strong>The complete DeepSeek Harness experience in a lightweight, self-contained Windows desktop app.</strong><br>
  No separate Node.js or DSH install. The first launch works offline, with a plugin market, Skills, and MCP management included.
</p>

<p align="center">
  <a href="https://dsh.cubee.chat/go/windows/latest?source=github_readme"><strong>Download for Windows x64</strong></a> ·
  <a href="https://dsh.cubee.chat/en/">Website</a> ·
  <a href="https://github.com/licn9901-arch/deepseek-harness-desktop/releases">Releases</a> ·
  <a href="https://github.com/licn9901-arch/deepseek-harness-desktop/issues/new/choose">Report an issue</a>
</p>

<p align="center">
  <a href="https://github.com/licn9901-arch/deepseek-harness-desktop/actions/workflows/windows-ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/licn9901-arch/deepseek-harness-desktop/windows-ci.yml?branch=main&label=Windows%20CI" alt="Windows CI"></a>
  <a href="https://github.com/licn9901-arch/deepseek-harness-desktop/releases"><img src="https://img.shields.io/github/v/release/licn9901-arch/deepseek-harness-desktop?include_prereleases&label=version" alt="Current version"></a>
  <a href="https://github.com/licn9901-arch/deepseek-harness-desktop/releases"><img src="https://img.shields.io/github/downloads/licn9901-arch/deepseek-harness-desktop/total?label=downloads" alt="Total downloads"></a>
  <img src="https://img.shields.io/badge/platform-Windows%20x64-0078D4" alt="Windows x64">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-1f883d" alt="MIT License"></a>
</p>

> [!IMPORTANT]
> This is a community-maintained project. It is not an official DeepSeek product and does not represent DeepSeek.

![Plugin search, workspace, and installation in DeepSeek Harness Desktop](https://dsh.cubee.chat/assets/plugin-market-live.v1.png)

## Current Preview

| Item | Details |
|---|---|
| Version | `v0.1.0-preview.12`; the next public channel will move to Beta |
| Installer | `87.57 MiB`, including Node.js, DSH, Market, pnpm, and bundled plugins |
| System | Windows 10 22H2 / Windows 11 x64 with the system WebView2 runtime |
| First launch | Works offline; no separate Node.js, DSH, or pnpm installation |
| Signing | No Authenticode yet; verify the SHA-256 before installation |

## Start in Three Steps

1. Get the installer from the [Windows download page](https://dsh.cubee.chat/en/download/windows/) and follow the SHA-256 verification instructions.
2. Install and launch the app, then wait for the local DSH Host to become ready.
3. Select a project, reference files, run a task, and inspect file, Diff, or GenUI results.

## Built for These Workflows

- Use DSH directly on Windows without maintaining Node.js, pnpm, and a separate web service.
- Keep conversations, files, Git, a local terminal, the plugin market, and interactive GenUI in one workspace.
- Manage project Skills and MCP connections while retaining local-first, inspectable runtime boundaries.
- Continue tasks after closing the window, then restore or restart the DSH Host from the system tray.

## Included Capabilities

- Starts local `dsh web` on a random loopback port and validates its readiness URL before loading WebView2.
- Single instance, close to tray, serialized Host restart, and explicit-exit cleanup.
- Plugin Market, Better Sidebar, GenUI, Skin Center, project memory, ModLens, and Skills/MCP management.
- Pinned offline runtime and plugins without overwriting same-name user installations or user-disabled state.
- Host pages receive no Tauri capability; external HTTP/HTTPS links open in the system browser.

Third-party plugins have the same Host permissions as the desktop application. Package signature verification, permission manifests, and process-level sandboxing are not currently available. MCP `env` and `headers` values are stored in plaintext at `~/.dsh/mcp.json`. Review plugin sources and install scripts before installation.

## Develop from Source

Requirements: PowerShell 7, Node.js `22.22.3`, Rust `1.94.1`, MSVC C++ Build Tools, and WebView2.

```powershell
git clone https://github.com/licn9901-arch/deepseek-harness-desktop.git
Set-Location deepseek-harness-desktop
npm ci
.\dev.cmd
```

```powershell
npm run validate:icons
npm run lint
npm test
npm run coverage
```

Maintainer contracts for release builds, payloads, pnpm compatibility, PID lifecycle, security boundaries, and upgrade gates live in:

- [Testing and release gates](docs/testing.md)
- [Desktop runtime and installer optimization](docs/runtime-packaging-optimization.md)
- [Release checklist](docs/release-checklist.md)

## Upstream Relationship and License

This repository maintains the Tauri shell, self-contained runtime, Windows installer, and security boundaries. It does not rewrite the DSH Web UI or Agent core. DSH, Market, pnpm, the web frontend, and third-party plugins are pinned by lockfile and retain their upstream licenses. The desktop shell uses the [MIT License](LICENSE).

If the project solves a Windows DSH installation or workflow problem for you, Star the repository. More importantly, complete one real task and report any installation or startup issue.
