# 预览版发布检查表

## 代码与依赖

- [ ] 版本号与标签一致，格式为 `v0.1.0-preview.7`。
- [ ] `runtime.lock.json`、Node SHA-256、DSH、DSH Market 与 pnpm package lock 完全匹配。
- [ ] `plugins.lock.json`、GitHub 归档 SHA-256、npm integrity 与插件 package lock 完全匹配。
- [ ] staged 边界不包含 `resources`、`.runtime-cache`、`target`、日志、本机路径或安装包。
- [ ] 主项目、`runtime-host` 与 `plugin-runtime` 三组 `npm audit` 通过。

## 自动化门禁

- [ ] `cargo fmt --check`、Clippy `-D warnings`、`cargo test --locked` 通过。
- [ ] 核心 Rust 库行覆盖率不低于 80%。
- [ ] fake Host 冒烟验证就绪、单实例、关闭到托盘和显式退出。
- [ ] fake Host 启动恢复验证核心先就绪、CoreReady 后崩溃、插件永不完成且最多重试一次。
- [ ] 隔离的 pnpm 10 profile 通过 Market 完成 `dsh-pet` 安装与卸载，且不触碰真实 `$DSH_HOME`。
- [ ] 安装器冒烟验证安装、内置运行时启动、生命周期、卸载和无残留进程。
- [ ] NSIS 已按 digest 预置插件 store，卸载不删除该缓存或用户 profile。
- [ ] 正式安装版热启动 20 次 CoreReady P95 不超过 8 秒，冷启动 3 次均不超过 20 秒。
- [ ] 缺少任一内置运行时关键文件时构建失败。
- [ ] 原始 `dshmarket@1.9.0` 客户端注册 ID 与活动 Cordis entry 一致，不存在桌面改名副本。
- [ ] `dsh --profile web --dump-config --patch policy/dsh-market.patch.yml` 中 `dsh-market` 处于活动状态，且 `profile=web`、`allowRestart=false`。
- [ ] 托盘重启后 PID 和动态端口更新，WebView 恢复且不误报旧 Host 异常退出。
- [ ] 9 个托管 bundle、GenUI Skill、Skin Center、关键客户端资产和 Windows x64 PTY 缺少任一文件时构建失败。
- [ ] “预置插件”只修改 web profile bundles；受保护和未知 bundle 均被拒绝，并发写不丢更新。
- [ ] “主题”一级入口可列出皮肤，试穿、应用、恢复默认与重启持久化通过。
- [ ] Hindsight 未 opt-in 时无请求，凭据不回显；ModLens 与 JSON-only Skills/MCP Manager 完成配置后可用。
- [ ] 启动页无脚本错误，Canvas 非空且持续运动，1280x800 与 960x600 无文字或控件重叠。

## 发布产物

- [ ] NSIS 安装包、`.sha256`、`third-party-licenses.json`、第三方声明和构建摘要齐全。
- [ ] Release 标记为 prerelease，并明确社区非官方、Windows x64、内置版本和未签名提示。
- [ ] Windows 10 22H2 与 Windows 11 x64 干净环境完成离线首次启动和任务测试。
- [ ] 安装、升级、回滚和卸载均不删除 DSH 用户会话与配置。
- [ ] 本机全部门禁通过后再创建 Git 标签和 GitHub prerelease，不依赖 GitHub Actions。
