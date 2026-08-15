# 贡献指南

本项目是 DeepSeek Harness 的非官方 Windows 桌面封装。提交改动前，请先确认改动仅涉及桌面生命周期、运行时、构建发布或配套文档，不修改 DSH Web UI 与 Agent 核心能力。

## 本地验证

在 PowerShell 7 中运行：

```powershell
npm ci
npm run validate:icons
Push-Location src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
Pop-Location
npm audit
```

新增 Rust 类型、方法和关键分支应提供简体中文注释。提交前检查 `git diff --check` 与 staged 文件边界，不要提交 `target`、运行时暂存文件、日志、安装包缓存或本机绝对路径。
