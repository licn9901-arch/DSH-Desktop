# 预览版发布检查表

## 代码与依赖

- [ ] 版本号与标签一致，格式为 `v0.1.0-preview.2`。
- [ ] `runtime.lock.json`、Node SHA-256、DSH package lock 完全匹配。
- [ ] `plugins.lock.json`、GitHub 归档 SHA-256、npm integrity 与插件 package lock 完全匹配。
- [ ] staged 边界不包含 `resources`、`.runtime-cache`、`target`、日志、本机路径或安装包。
- [ ] 主项目、`runtime-host` 与 `plugin-runtime` 三组 `npm audit` 通过。

## 自动化门禁

- [ ] `cargo fmt --check`、Clippy `-D warnings`、`cargo test --locked` 通过。
- [ ] 核心 Rust 库行覆盖率不低于 80%。
- [ ] fake Host 冒烟验证就绪、单实例、关闭到托盘和显式退出。
- [ ] 安装器冒烟验证安装、内置运行时启动、生命周期、卸载和无残留进程。
- [ ] 缺少任一内置运行时关键文件时构建失败。
- [ ] 四个插件、Skin Center、本地 GenUI 资产和 Windows x64 PTY 缺少任一文件时构建失败。

## 发布产物

- [ ] NSIS 安装包、`.sha256`、`third-party-licenses.json`、第三方声明和构建摘要齐全。
- [ ] Release 标记为 prerelease，并明确社区非官方、Windows x64、内置版本和未签名提示。
- [ ] Windows 10 22H2 与 Windows 11 x64 干净环境完成离线首次启动和任务测试。
- [ ] 安装、升级、回滚和卸载均不删除 DSH 用户会话与配置。
- [ ] 本机全部门禁通过后再创建 Git 标签和 GitHub prerelease，不依赖 GitHub Actions。
