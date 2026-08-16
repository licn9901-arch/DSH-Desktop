# DeepSeek Harness Desktop v0.1.0-preview.5

这是 Windows x64 社区预览版。本项目不是 DeepSeek 官方产品，也不代表 DeepSeek 官方立场。

## 本版内容

- 内置 Market 升级到原始 `dshmarket@1.9.0`，由受保护的桌面 Runtime Services 提供 profile
  与固定 pnpm。已有 pnpm 9/10/11 profile 会选择相同 major，新 profile 默认使用 11。
- 设置新增“项目记忆”一级入口，可配置 Hindsight Cloud 或自托管地址、只写 API Token，
  并从现有工作区或目录选择器显式启用项目。未启用项目保持完全不记录。
- Skills/MCP Manager 升级为桌面维护版 `@cubee-slide/skills-mcp-manager@0.2.3`：重新设计 Skills 列表、导入、详情、删除确认、
  MCP 主从布局与操作状态，统一按钮和开关；MCP 配置只保留 JSON 编辑器。
- 设置保留“预置插件”和“主题”一级入口。Runtime Services、Market 与桌面设置适配器不可停用，
  可停用插件的状态在升级和 Host 重启后继续保留。
- Skin Center 使用上游 `0.1.17`，启动时会迁移旧预览版遗留的非法 Skin patch，
  保留已应用的主题管理块。
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
