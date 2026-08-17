# DeepSeek Harness Desktop v0.1.0-preview.7

这是 Windows x64 社区预览版。本项目不是 DeepSeek 官方产品，也不代表 DeepSeek 官方立场。

## 本版内容

- 原生窗口标题栏现在直接显示桌面版版本号，方便截图、反馈和排查时确认实际运行版本。
- 插件市场“源码”和 README 中的外链新窗口请求现在交给系统默认浏览器，主 WebView 保持在当前页面。
- Market 安装公共 GitHub 依赖时增加 HTTPS 兜底，不再要求用户必须预先配置 GitHub SSH 密钥。
- Market 安装、更新或卸载插件后只保留本次操作产生的 bundle 变化，不会顺带启用 profile 中已有的伴随依赖。
- Skin Center 伴随包只用于 Node 解析，不再作为顶层 bundle 重复加载；主题入口、应用状态和 Host 重启保持稳定。
- 设置继续提供“预置插件”“项目记忆”“主题”“技能与 MCP”和“插件市场”入口。
- 内置 `dshmarket@1.9.0`，并固定离线 pnpm 9、10、11；已有 profile 使用相同 pnpm major，新 profile 默认使用 11。
- Skills/MCP Manager 使用桌面维护版 `@cubee-slide/skills-mcp-manager@0.2.3`，MCP 配置只保留 JSON 编辑器。
- GenUI Skill 继续作为 DSH 用户级全局 Skill 托管；用户自有、修改或删除的同名 Skill 不覆盖。
- ModLens 保持 `3.16.7`，不预置密钥或视觉端点。

## 安全边界

- Hindsight Token 写入 `$DSH_HOME/.credentials.yaml`，设置页只读取配置状态，不回显内容。
- Hindsight 始终保持 `harnesses.dsh.optInOnly=true`，只有显式选择的绝对路径会写入清单。
- MCP `env` 与 `headers` 仍以明文保存在 `~/.dsh/mcp.json`。
- pnpm 9/10 只为已有 profile 兼容保留；当前 npm audit 对这些旧 major 报告无可用修复的高危公告。
- 第三方插件与桌面应用拥有相同主机权限，目前没有插件签名或进程级沙箱。

## 安装提示

- 支持 Windows 10 22H2 / Windows 11 x64。
- 安装包没有 Authenticode 签名，SmartScreen 可能显示未知发布者。
- 请先核对随 Release 提供的 `.sha256` 文件。
- 设置变更后，请从托盘选择“重启 DSH 服务”。
- 卸载不会删除 DSH 用户会话、profile、插件状态、Skill 或第三方配置。
