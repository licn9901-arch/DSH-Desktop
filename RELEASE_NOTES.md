# DeepSeek Harness Desktop v0.1.0-preview.4

这是 Windows x64 社区预览版。本项目不是 DeepSeek 官方产品，也不代表 DeepSeek 官方立场。

## 本版内容

- 修复 `dshmarket-desktop` 客户端仍以 `dshmarket` 注册，导致打包应用启动时报
  `loaded without registering "dshmarket-desktop"` 的问题。桌面 runtime 现在生成注册名适配副本，
  staging 会严格校验除注册 ID 外的内容与上游 Market 完全一致。
- 启动页改为离线 DeepSeek“数字深海”动画，包含鲸鱼标志、五层汇聚粒子波、呼吸光源、
  极弱扩散光波和精简启动状态；支持最小窗口与 `prefers-reduced-motion`。
- 内置 Node.js `22.22.3` 与 `@deepseek-ai/dsh 0.1.0-rc.6`，首次启动无需联网。
- 离线托管 8 个 bundle：原有 At File、GenUI、Better Sidebar，桌面设置适配器，
  Skins `0.1.17`、Hindsight `0.3.4`、ModLens `3.16.7` 与 Skills/MCP Manager `0.1.3`。
- 设置新增“预置插件”一级入口。开关只原子修改 web profile 的 bundles，不卸载文件、不调用
  pnpm；官方 bundle、Market、桌面控制 bundle 和未知用户 bundle 均受保护。
- 设置新增“主题”一级入口，继续复用上游 Skin Center 的试穿、应用、恢复和 Host API。
- GenUI Skill 首次安装到 `$DSH_HOME/skills/genui/SKILL.md`；未修改的托管文件可升级，
  用户自有、修改或删除的同名 Skill 均保留。
- Hindsight 默认只允许显式 opt-in 项目，初始清单为空；已有
  `~/.hindsight/coding-agent.json` 完全保留。
- ModLens 不预置密钥或视觉端点。Skills/MCP Manager 的 MCP `env` 与 `headers`
  会以明文保存在 `~/.dsh/mcp.json`。
- 插件 profile、链接、GenUI Skill 与 Hindsight 首次配置共享启动事务；Host 失败时一并回滚。

## 插件权限提示

- Better Sidebar 可读写工作区文件、执行 Git，并创建本机 PTY 进程。
- Skin Center 应用皮肤时会写入 `$DSH_HOME/cordis.patch.yml`。
- GenUI action 会把用户交互作为消息发送给当前模型。
- Hindsight 仅在用户配置服务且显式 opt-in 项目后记录或上传。
- ModLens 可把用户选择的图片发送到用户配置的视觉服务。
- Skills/MCP Manager 可删除 Skill、管理真实 MCP 连接，并明文保存 MCP 连接参数。

## 安装提示

- 支持 Windows 10 22H2 / Windows 11 x64。
- 安装包没有 Authenticode 签名，SmartScreen 可能显示未知发布者。
- 请先核对随 Release 提供的 `.sha256` 文件。
- 预置插件开关保存后，请从托盘选择“重启 DSH 服务”。
- 不支持自动更新；升级或回滚请手动安装对应 Release。
- 卸载不会删除 DSH 用户会话、profile、插件 marker、Skill 或第三方配置。
