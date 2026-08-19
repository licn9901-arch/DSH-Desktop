# 贡献指南

本项目是 DeepSeek Harness 的非官方 Windows / macOS 桌面封装。提交改动前，请先确认改动仅涉及桌面生命周期、运行时、构建发布或配套文档，不修改 DSH Web UI 与 Agent 核心能力。

## 本地验证

在 PowerShell 7 中运行：

```powershell
npm ci
npm run validate:icons
npm run lint
npm test
npm audit
Push-Location runtime-host
npm audit --omit=dev
Pop-Location
Push-Location plugin-runtime
npm audit --omit=dev
Pop-Location
npm run coverage
```

新增 Rust 类型、方法和关键分支应提供简体中文注释。提交前检查 `git diff --check` 与 staged 文件边界，不要提交 `target`、运行时暂存文件、日志、安装包缓存或本机绝对路径。

修改运行时或打包流程后还应运行 `stage/verify:runtime`、`stage/verify:plugins`、`package/verify:payload`
和 Windows 安装器冒烟。payload 灰度完成前，默认 `npm run build` 必须保持 legacy。公开发布前必须在本机
完成发布检查表，再推送 `v*` 标签并用 `gh release create --prerelease` 上传校验后的产物。

Apple Silicon Mac 的构建必须执行 `npm run release:gate:macos`。不要直接发布
`src-tauri/target/release/bundle` 中的文件；只有 `.release-work/macos/gated` 中同时带 SHA-256 与通过报告的
DMG 可以进入 Release。Mac 相关改动不得改变 Windows 默认 `build`、payload、NSIS 或安装器门禁。
