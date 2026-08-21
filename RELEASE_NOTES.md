# DeepSeek Harness Desktop v0.1.0-preview.13

这是 Windows x64 社区预览版。本项目不是 DeepSeek 官方产品，也不代表 DeepSeek 官方立场。

## 本版升级

- DSH 内核升级到 `0.1.0-rc.8`，使用内核原生的文件与会话 `@` 引用。
- DSH Market 升级到 `1.17.1`。
- 内置插件升级到 GenUI `0.8.6`、Better Sidebar `0.14.0`、Skin Center `0.2.6`、Hindsight `0.4.1` 和 ModLens `3.22.1`；Skills/MCP Manager 保持 `0.2.4`。
- 不再内置 `dsh-at-file`。桌面版只移除自己托管的旧 bundle；用户自行安装的同名依赖保持用户所有权。
- 托盘新增“项目官网”“反馈问题”“检查更新”，所有入口都是经过既有外部 URL 白名单校验的编译期 HTTPS 地址。

## 数据兼容提醒

- DSH `rc.8` 调整了 SQLite 存储结构，新内核写入的数据不能假定可由旧内核读取。
- 手动安装旧桌面版只会回退应用与 payload，不等于完成数据回滚。
- 发布前必须使用隔离的数据副本完成 `preview.12` 到 `preview.13` 的会话读取、新建、分叉、继续对话和候选失败回退验证。

## 安装提示

- 支持 Windows 10 22H2 / Windows 11 x64。
- 安装包没有 Authenticode 签名，SmartScreen 可能显示未知发布者。
- 请先核对随 Release 提供的 `.sha256` 文件。
- 安装包未提供 Authenticode 签名，SmartScreen 可能显示“未知发布者”；这不影响 SHA-256 校验的必要性。
- 卸载不会删除 DSH 用户会话、profile、插件状态、Skill 或第三方配置。
