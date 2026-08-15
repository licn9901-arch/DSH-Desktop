# DeepSeek Harness Desktop v0.1.0-preview.1

这是首个 Windows x64 社区预览版。

> 本项目是社区维护的桌面封装，不是 DeepSeek 官方产品，也不代表 DeepSeek 官方立场。

## 包含内容

- 内置 Node.js `22.22.3`。
- 内置 `@deepseek-ai/dsh 0.1.0-rc.6`。
- 单实例、关闭到托盘、显式退出清理 Host。
- 严格就绪地址解析、同源 WebView 导航和外链系统浏览器处理。
- 脱敏轮转日志和自包含 NSIS 安装包。

## 安装提示

- 支持 Windows 10 22H2 / Windows 11 x64。
- 安装包没有 Authenticode 签名，SmartScreen 可能显示未知发布者。
- 请先核对随 Release 提供的 `.sha256` 文件。
- 首版不支持自动更新；升级或回滚请手动安装对应 Release。
- 卸载不会删除 DSH 用户会话和配置。
