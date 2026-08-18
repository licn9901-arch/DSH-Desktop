# DeepSeek Harness Desktop v0.1.0-preview.8

这是 Windows x64 社区预览版。本项目不是 DeepSeek 官方产品，也不代表 DeepSeek 官方立场。

## 本版内容

- 原生窗口标题栏现在直接显示桌面版版本号，方便截图、反馈和排查时确认实际运行版本。
- 插件市场“源码”和 README 中的外链新窗口请求现在交给系统默认浏览器，主 WebView 保持在当前页面。
- 修复 candidate 预校验后把 `http://tauri.localhost` 误判为外链并拉起浏览器的问题；Tauri 内置 origin 始终留在主 WebView。
- Market 安装公共 GitHub 依赖时增加 HTTPS 兜底，不再要求用户必须预先配置 GitHub SSH 密钥。
- Market 安装、更新或卸载插件后只保留本次操作产生的 bundle 变化，不会顺带启用 profile 中已有的伴随依赖。
- Skin Center 伴随包只用于 Node 解析，不再作为顶层 bundle 重复加载；主题入口、应用状态和 Host 重启保持稳定。
- 设置继续提供“预置插件”“项目记忆”“主题”“技能与 MCP”和“插件市场”入口。
- 内置 `dshmarket@1.10.0` 与唯一的 `pnpm@10.34.5`；三组 audit 为零漏洞，pnpm 9/10/11 历史 profile 已通过真实兼容与字节恢复矩阵。
- 新增确定性三 ZIP payload、原子 candidate/active/previous 状态机和快速 NSIS preview；完成灰度门禁前默认构建仍使用 legacy。
- 健康启动不再因 Windows junction 的 verbatim/长路径表示差异重复重建 10 个插件链接；插件准备与 WebView2 初始化并行。
- PDB、source map、类型、测试和示例源码从默认安装器移入独立 debug symbols 产物。
- Skills/MCP Manager 使用桌面维护版 `@cubee-slide/skills-mcp-manager@0.2.3`，MCP 配置只保留 JSON 编辑器。
- GenUI Skill 继续作为 DSH 用户级全局 Skill 托管；用户自有、修改或删除的同名 Skill 不覆盖。
- ModLens 保持 `3.16.7`，不预置密钥或视觉端点。

## 安全边界

- Hindsight Token 写入 `$DSH_HOME/.credentials.yaml`，设置页只读取配置状态，不回显内容。
- Hindsight 始终保持 `harnesses.dsh.optInOnly=true`，只有显式选择的绝对路径会写入清单。
- MCP `env` 与 `headers` 仍以明文保存在 `~/.dsh/mcp.json`。
- profile 内控制文件和依赖树支持失败回滚；第三方安装脚本在 profile 外产生的副作用不属于事务边界。
- 第三方插件与桌面应用拥有相同主机权限，目前没有插件签名或进程级沙箱。

## 安装提示

- preview.7 仅作为 SHA-256 为 `e331e628b07bf574e823610324130c258d77ed1e57113b59426feed1a3a9d3d9` 的 legacy 升级基线。
- 本版是第一轮公开 payload preview；默认 `npm run build` 仍指向 legacy，Release 安装器必须显式使用 `build:payload` 并通过完整门禁。
- preview.8 完整 payload 门禁已通过；最终安装器 SHA-256 为 `c16498e160cc94b73082edf249353d54e2b6a3129920a2587963815f7036ba5e`。
- 支持 Windows 10 22H2 / Windows 11 x64。
- 安装包没有 Authenticode 签名，SmartScreen 可能显示未知发布者。
- 请先核对随 Release 提供的 `.sha256` 文件。
- 设置变更后，请从托盘选择“重启 DSH 服务”。
- 卸载不会删除 DSH 用户会话、profile、插件状态、Skill 或第三方配置。
