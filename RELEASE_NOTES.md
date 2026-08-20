# DeepSeek Harness Desktop v0.1.0-preview.12

这是 Windows x64 社区预览版。本项目不是 DeepSeek 官方产品，也不代表 DeepSeek 官方立场。

## 本版修复

- 修复从旧 payload 或本地体验版升级时，profile 中的 `dshmarket` 仍指向已清理 runtime，导致首次启动报 `ERR_MODULE_NOT_FOUND` 的问题。
- 仅迁移明确位于桌面托管 runtime 根目录中的 Market 链接；npm 版本和用户自定义路径仍保持用户所有权，不会被桌面端覆盖。
- 修复首次 candidate 校验失败且没有旧 active runtime 时过早清空 candidate，导致第二次启动误走 legacy 安装目录并报告缺少 `node/node.exe` 的问题。
- 首次启动失败时保留已校验并展开的 candidate，后续启动仍使用同一 payload，不再退回 payload 安装器未携带的 legacy 根目录。

## 安装提示

- 支持 Windows 10 22H2 / Windows 11 x64。
- 安装包没有 Authenticode 签名，SmartScreen 可能显示未知发布者。
- 请先核对随 Release 提供的 `.sha256` 文件。
- 卸载不会删除 DSH 用户会话、profile、插件状态、Skill 或第三方配置。
