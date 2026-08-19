# DeepSeek Harness Desktop v0.1.0-preview.10

这是 Windows x64 社区预览版。本项目不是 DeepSeek 官方产品，也不代表 DeepSeek 官方立场。

## 本版内容

- 默认 `npm run build` 在连续两轮公开 payload 门禁通过后切换到确定性 payload 构建；legacy 构建链继续保留，供回归和升级验证使用。
- 主题中心升级为独立的 `@linxin666/dsh-client-ui-skin-center@0.2.2`，单包交付全部内置皮肤，不再安装已退役的 `dsh-skins` 聚合载具。
- 发布构建会把设置页画廊预览确定性缩放为最大 480px 的调色板 PNG；主题运行时使用的原始背景资产保持不变。
- 从旧版升级时，仅迁移仍由桌面 marker 管理的主题载具，并把原有启用状态转交给新的 Skin Center；用户自行安装的同名依赖不受影响。
- 主题试穿、应用和恢复现在即时生效，无需重启 Host；设置页会在切换时清理旧 bundle，避免重复注册。
- 修复插件 fast path 对伴随依赖 bundle 状态判断不完整的问题，异常激活的 transitive-only bundle 会触发修复流程。
- Skin Center 可读取用户显式选择的 Wallpaper Engine 或本机媒体目录，并将当前选择写入 `$DSH_HOME/skin-center-active.json`。
- 最终安装器为 91,814,862 字节（87.56 MiB），SHA-256 为 `ee7f9a4613a920a0b76eec5af696a2e0aebee21d856e188da87bcfb734fd52df`。
- 三 ZIP 共 93,546,324 字节，展开为 254,073,088 字节、12,972 个文件；两次强制构建的 manifest 与三 ZIP 逐字节一致。
- 6/6 升级矩阵与三组零漏洞 audit 通过；20 对 warm 启动 P95 为 legacy 11,628 ms、payload 9,707 ms，低于 12,210 ms 门限。

## 安装提示

- 支持 Windows 10 22H2 / Windows 11 x64。
- 安装包没有 Authenticode 签名，SmartScreen 可能显示未知发布者。
- 请先核对随 Release 提供的 `.sha256` 文件。
- 卸载不会删除 DSH 用户会话、profile、插件状态、Skill 或第三方配置。
