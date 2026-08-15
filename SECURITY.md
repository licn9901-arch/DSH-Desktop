# 安全政策

## 支持范围

当前只对最新 GitHub prerelease 提供安全修复。预览版接口与存储行为可能调整，请在升级前备份重要工作。

## 报告漏洞

请通过 GitHub 仓库的 Private vulnerability reporting 提交安全问题，不要在公开 Issue 中披露可利用细节、token、Cookie、密码、会话文件或未经脱敏的日志。

报告应包含受影响版本、Windows 版本、复现步骤、预期与实际行为、影响范围，以及必要的脱敏诊断信息。维护者确认前请不要公开利用代码。

## 安全边界

- WebView 只允许已验证的本地 Host 原点，外部 HTTP/HTTPS 链接由系统浏览器打开。
- 远程 Host 页面不获得 Tauri capability。
- release 构建只执行锁文件指定的内置 Node 与 DSH。
- 首个预览版未使用 Authenticode 签名，下载后应核对 Release 的 SHA-256 文件。
