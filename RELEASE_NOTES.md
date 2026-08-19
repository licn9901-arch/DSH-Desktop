# DeepSeek Harness Desktop v0.1.0-preview.11

这是 Windows x64 社区预览版。本项目不是 DeepSeek 官方产品，也不代表 DeepSeek 官方立场。

## 本版内容

- 桌面设置包从 `@dsh-desktop/theme-settings` 迁移为可选的 `@dsh-desktop/settings`，保留“预置插件”和“项目记忆”，删除旧“主题”入口及 Skin Center 容器。
- 设置插件默认安装但允许卸载；完整重启、托盘重启和后续协调不会恢复用户已卸载的设置包，重新安装时继续读取原配置。
- 托盘“重启 DSH 服务”现在执行完整插件事务：启动成功后提交，失败时回滚 profile 与链接并只恢复启动一次。
- Windows 原生目录选择器绑定桌面主窗口，添加工作区时不再把 Node 显示为独立任务栏窗口。
- Skills/MCP Manager 升级到 `0.2.4`，使用 Skin Center 语义主题、紧凑控件和可键盘操作的状态筛选。
- 发布报告绑定候选 Git commit，统一门禁拒绝脏工作区、跨提交旧报告和非官方 npm registry 审计结果。

## 安装提示

- 支持 Windows 10 22H2 / Windows 11 x64。
- 安装包没有 Authenticode 签名，SmartScreen 可能显示未知发布者。
- 请先核对随 Release 提供的 `.sha256` 文件。
- 卸载不会删除 DSH 用户会话、profile、插件状态、Skill 或第三方配置。
