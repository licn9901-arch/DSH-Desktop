# DeepSeek Harness Desktop v0.1.0-preview.2

这是 Windows x64 社区预览版。本项目不是 DeepSeek 官方产品，也不代表 DeepSeek 官方立场。

## 本版内容

- 内置 Node.js `22.22.3` 与 `@deepseek-ai/dsh 0.1.0-rc.6`，首次启动无需联网。
- 离线内置 `dsh-at-file 0.6.0`、`@omdsh-dev/dsh-genui 0.8.4`、
  `dsh-better-sidebar 0.12.2`、`@linxin666/dsh-skins 0.1.16`。
- 插件发布物固定 SHA-256/npm integrity，构建时禁用安装脚本；profile 更新和 junction 创建可回滚。
- 用户自行安装的同名插件与主动禁用状态优先；旧 `@dsh-external/dsh-side-panel` 只从活动 bundle 移除。
- Better Sidebar 首次托管安装会关闭 HTTP/HTTPS 接管；插件故障会回滚并只重试一次核心 DSH。
- 保留单实例、关闭到托盘、显式退出清理 Host、同源导航和外链系统浏览器安全边界。

## 插件权限提示

- Better Sidebar 可读写工作区文件、执行 Git，并创建本机 PTY 进程。
- Skin Center 应用皮肤时会写入 `$DSH_HOME/cordis.patch.yml`。
- GenUI action 会把用户交互作为消息发送给当前模型。
- 本版不提供动态安装、在线更新或插件权限确认界面。

## 安装提示

- 支持 Windows 10 22H2 / Windows 11 x64。
- 安装包没有 Authenticode 签名，SmartScreen 可能显示未知发布者。
- 请先核对随 Release 提供的 `.sha256` 文件。
- 不支持自动更新；升级或回滚请手动安装对应 Release。
- 卸载不会删除 DSH 用户会话、profile、插件 marker 或配置。
