# DeepSeek Harness Desktop

<p align="center">
  <strong>简体中文</strong> · <a href="README.en.md">English</a>
</p>

<p align="center">
  <img src="src-tauri/icons/icon.png" width="112" height="112" alt="DeepSeek Harness Desktop 图标">
</p>

<p align="center">
  <strong>把完整 DeepSeek Harness 装进一个轻量、自包含的 Windows 桌面应用。</strong><br>
  无需预装 Node.js 或 DSH，首次启动可离线；内置插件市场、Skills 与 MCP 管理。
</p>

<p align="center">
  <a href="https://dsh.cubee.chat/go/windows/latest?source=github_readme"><strong>下载 Windows x64</strong></a> ·
  <a href="https://dsh.cubee.chat/">项目官网</a> ·
  <a href="https://github.com/licn9901-arch/deepseek-harness-desktop/releases">发布记录</a> ·
  <a href="https://github.com/licn9901-arch/deepseek-harness-desktop/issues/new/choose">反馈问题</a>
</p>

<p align="center">
  <a href="https://github.com/licn9901-arch/deepseek-harness-desktop/releases"><img src="https://img.shields.io/github/v/release/licn9901-arch/deepseek-harness-desktop?include_prereleases&label=version" alt="当前版本"></a>
  <a href="https://github.com/licn9901-arch/deepseek-harness-desktop/releases"><img src="https://img.shields.io/github/downloads/licn9901-arch/deepseek-harness-desktop/total?label=downloads" alt="累计下载"></a>
  <img src="https://img.shields.io/badge/platform-Windows%20x64-0078D4" alt="Windows x64">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-1f883d" alt="MIT License"></a>
</p>

> [!IMPORTANT]
> 本项目由社区维护，不是 DeepSeek 官方产品，也不代表 DeepSeek 官方立场。

![DeepSeek Harness Desktop 从选择项目、引用文件到执行任务和查看结果的完整流程](https://dsh.cubee.chat/assets/desktop-task-demo.v1.gif)

<p align="center"><sub>选择项目、引用文件、执行任务，并查看工具进度、GenUI 汇总和生成结果。</sub></p>

## 任务演示

![DeepSeek Harness Desktop 完成任务后的 GenUI 测试摘要](https://dsh.cubee.chat/assets/desktop-genui-result.v1.webp)

<p align="center"><sub>任务完成后，修改文件与 17 项测试结果会集中显示在 GenUI 摘要中。</sub></p>

![DeepSeek Harness Desktop 生成并预览的网页结果](https://dsh.cubee.chat/assets/desktop-task-result.v1.webp)

<p align="center"><sub>生成的网页可以直接在工作区中预览。</sub></p>

## 当前预览版

| 项目 | 内容 |
|---|---|
| 版本 | `v0.1.0-preview.12`，下一公开通道将转为 Beta |
| 安装包 | `87.57 MiB`，自包含 Node.js、DSH、Market、pnpm 与预置插件 |
| 系统 | Windows 10 22H2 / Windows 11 x64，依赖系统 WebView2 |
| 首次启动 | 无需联网，不要求预装 Node.js、DSH 或 pnpm |
| 签名 | 暂无 Authenticode；请核对 SHA-256 后安装 |

## 三步开始

1. 从[官网 Windows 下载页](https://dsh.cubee.chat/download/windows/)获取安装包，并按页面说明核对 SHA-256。
2. 安装后启动应用，等待本地 DSH Host 就绪。
3. 选择项目目录，在对话中引用文件、执行任务并检查工具进度、修改文件、测试或 GenUI 结果。

## 皮肤主题与扩展能力

![DeepSeek Harness Desktop 四种皮肤主题](https://dsh.cubee.chat/assets/theme-gallery-local.v1.webp)

<p align="center"><sub>在 Skin Center 中预览主题效果，满意后再应用到工作区。</sub></p>

![DeepSeek Harness Desktop 插件市场搜索 GenUI 并显示安装状态](https://dsh.cubee.chat/assets/plugin-market-local.v2.webp)

![DeepSeek Harness Desktop Skills 列表与 MCP 设置](https://dsh.cubee.chat/assets/skills-mcp-local.v2.webp)

## 适合这些工作

- 希望在 Windows 上直接使用 DSH，又不想维护 Node.js、pnpm 和 Web 服务环境。
- 需要在一个工作区中使用对话、文件、Git、本地终端、插件市场和可交互 GenUI。
- 需要管理项目 Skills 与 MCP 连接，并保持本地优先的运行方式。
- 需要关闭窗口后继续任务，并从系统托盘恢复或重启 DSH Host。

## 开箱能力

- 本地 `dsh web` 通过随机回环端口启动，严格校验就绪地址后才载入 WebView2。
- 单实例、关闭到托盘、Host 串行重启和显式退出清理。
- 插件市场、Better Sidebar、GenUI、皮肤中心、项目记忆、ModLens、Skills/MCP 管理。
- 固定版本的离线运行时与插件；不会覆盖用户自行安装的同名插件或主动禁用状态。
- Host 页面不获得 Tauri capability；外部 HTTP/HTTPS 链接交给系统浏览器。

第三方插件与桌面应用拥有相同 Host 权限。当前没有包签名验证、权限清单或进程级沙箱；MCP 配置中的 `env` 和 `headers` 会明文保存在 `~/.dsh/mcp.json`。安装第三方插件前请检查来源和安装脚本。

## 从源码运行

要求 PowerShell 7、Node.js `22.22.3`、Rust `1.94.1`、MSVC C++ Build Tools 与 WebView2。

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

发布构建、payload、pnpm 兼容、PID/生命周期、安全边界与升级门禁属于维护者接口，详见：

- [测试与发布门禁](docs/testing.md)
- [桌面运行时与安装包优化](docs/runtime-packaging-optimization.md)
- [发布检查清单](docs/release-checklist.md)

## 上游关系与许可证

本仓库维护 Tauri 桌面壳、自包含运行时、Windows 安装与安全边界，不重写 DSH Web UI 或 Agent 核心。DSH、Market、pnpm、Web 前端及第三方插件按 lockfile 固定版本，并分别遵循上游许可证。桌面壳使用 [MIT License](LICENSE)。

如果项目解决了你的 Windows DSH 安装或工作流问题，欢迎 Star 仓库；遇到安装或启动问题，请提交 Issue。
