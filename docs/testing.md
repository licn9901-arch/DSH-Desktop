# 测试与质量门禁

## 固定命令

在 PowerShell 7 中运行：

```powershell
npm ci
npm run validate:icons
npm run lint
npm test
npm run audit:release
npm run test:pnpm-compat
npm run stage:runtime
npm run verify:runtime
npm run stage:plugins
npm run verify:plugins
npm run package:payload
npm run verify:payload
npm run coverage
npm run smoke
npm run smoke:startup
git diff --check
```

`npm run coverage` 需要 `llvm-tools-preview` 与 `cargo-llvm-cov`，并以 80% 行覆盖率为最低门禁。
应用装配层 `desktop.rs`、`lib.rs`、`main.rs` 由 Windows 冒烟覆盖，不计入核心模块覆盖率。

## 测试层次

### Rust 与 Node 单元测试

Rust 测试覆盖 Host readiness、生命周期、导航、日志、插件事务和 payload 状态机。payload 契约至少覆盖：

- manifest/ZIP 摘要错误、截断 ZIP、文件数和展开大小超限；
- 绝对路径、父级路径、ADS、设备名、尾随点或空格、大小写冲突和重复目标；
- 并发 provision、中断 staging 恢复、candidate 晋升/拒绝、状态回滚与垃圾清理。

Runtime Services 的 Node fixture 覆盖固定 pnpm 10 PATH、Git 环境隔离、明确的 modules/hoist 错误识别、
控制文件字节快照、一次重建与一次重试，以及失败后旧依赖树和控制文件恢复。普通安装失败不得触发重建。

### 构建闭包

`verify-runtime` 和 `verify-plugins` 校验 schema 2 lock、入口、delivery、native external、资产和许可证。
`package:payload` 还会使用真实 Node 加载 `yaml`、OpenTelemetry、AWS Bedrock、`node-pty` 和 `sharp`，
然后生成三个 ZIP 与 manifest。`verify:payload` 使用运行时同一 Rust 模块重新验证。

相同输入的可复现性检查必须强制执行两次非缓存构建，并比较以下四个文件的 SHA-256：

```powershell
npm run verify:reproducibility
```

### 桌面生命周期

`smoke-test.ps1` 使用本次构建的桌面程序并只管理本次记录的 PID：

1. 等待 Host CoreReady/PluginsReady 和严格回环 URL；
2. 二次启动确认 single-instance，只聚焦现有窗口；
3. 关闭窗口确认应用隐藏到托盘且 Host 存活；
4. `--quit-existing` 进入正式退出路径；
5. 确认桌面与记录的 Host 进程树均结束。

`startup-scenarios.ps1` 使用 fake Host 覆盖核心先就绪、CoreReady 后崩溃和插件永不完成。失败清理禁止
扫描或批量终止其他 `node.exe`。

### Payload 安装器

```powershell
pwsh -NoProfile -File scripts/installer-smoke.ps1 `
  -Installer 'src-tauri\target\release\bundle\nsis\<installer>.exe' `
  -InstallRoot '.installer-smoke\payload' `
  -TimeoutSeconds 300 `
  -Payload `
  -SkipMarket
```

测试使用系统临时目录下的显式 runtime 根，验证四个安装资源、provision、真实 Host 与 9 个内置 bundle
readiness、candidate 晋升、single-instance、托盘生命周期、卸载 runtime 以及保留 `~/.dsh` sentinel。
测试 runtime 根会写入安装元数据；卸载前必须规范化并限制到固定临时目录前缀，不能接受任意删除路径。

移除 `-SkipMarket` 后，额外通过 Market HTTP API 安装和卸载 `dsh-pet`，覆盖用户插件真实链路。该项需要网络，
且必须使用隔离 profile。

### 升级、回滚与性能

preview.8 的发布矩阵命令如下；preview.9/10 还必须传入上一轮 payload 安装器：

```powershell
npm run test:upgrade-matrix -- `
  -LegacyInstaller '.deploy-artifacts\DeepSeek.Harness.Desktop_0.1.0-preview.7_x64-setup.exe' `
  -PayloadInstaller 'src-tauri\target\release\bundle\nsis\DeepSeek Harness Desktop_0.1.0-preview.8_x64-setup.exe'

npm run benchmark:compare -- `
  -LegacyInstaller '.deploy-artifacts\DeepSeek.Harness.Desktop_0.1.0-preview.7_x64-setup.exe' `
  -PayloadInstaller 'src-tauri\target\release\bundle\nsis\DeepSeek Harness Desktop_0.1.0-preview.8_x64-setup.exe'
```

启动对比使用统一外部墙钟，从 `Start-Process` 前开始，到两版都写出 `host ready` 为止；该日志表示 core
与 plugins 均已 ready。禁止将 preview.7 的 Host 内部耗时与 preview.8 的进程级 `core_ready` 直接比较。
健康 payload 启动会并行执行插件 prepare 与 WebView2 初始化；Windows junction 目标按 Win32 路径语义比较，
同目标的 verbatim、长路径或大小写差异不得触发链接重建。candidate、首次迁移和异常状态仍执行完整资源校验。

矩阵覆盖 clean install、legacy 到 payload、运行中升级、损坏 manifest/截断 ZIP、candidate readiness 失败、
垃圾清理和卸载。指定 `-PreviousPayloadInstaller` 后增加 payload 到 payload 的停止/运行中升级。旧 active 必须
保持可用，所有 install、`LOCALAPPDATA` 和 `DSH_HOME` 都位于独立系统临时目录，任何路径都不得删除用户 profile。
Preview.7 需要展开和预置 legacy 小文件树，安装阶段单独允许最多 15 分钟；payload 安装及 Host/readiness
仍使用命令传入的 180 秒门限，不能用 legacy 的宽限掩盖新版本启动超时。

安装器矩阵必须在专用、可丢弃的 Windows 用户中执行。`/D=`、`LOCALAPPDATA` 和 `DSH_HOME` 只能隔离
文件，不能改变 NSIS 固定的 HKCU 卸载键和 Shell 快捷方式名。四个安装器门禁脚本会在开始前检查
`dsh-desktop` 进程、产品注册表键、自动启动值和快捷方式，任一已存在就拒绝执行。自动化安装统一传入
`/NS`，WebView2 数据目录也限制在系统临时目录的固定测试结构；结束后只清理由本轮安装根拥有的
注册表与快捷方式。

`benchmark-compare.ps1` 安装两个正式安装器，复制相同 seed profile，各预热一次后交替执行 20 对 warm 启动；
报告所有样本、P50/P95、安装器 SHA、Windows/CPU 信息和各 3 次 cold。payload 相对 legacy 的劣化不得超过
5% 或 100 ms，两者取较大值。不得用单独测得的 payload P95 或 fake Host 数据替代发布结论。
preview.8 最终报告为 legacy P95 5,533 ms、payload P95 5,547 ms、门限 5,810 ms，门禁通过。

统一门禁命令会串行执行全部发布检查并输出 JSON/Markdown：

```powershell
npm run release:gate -- `
  -LegacyInstaller '<preview.7 legacy installer>' `
  -PayloadInstaller '<current payload installer>'
```

### macOS Apple Silicon 门禁

macOS 使用独立门禁，不修改 Windows 的默认 `build`、NSIS 配置、payload 脚本或安装器矩阵：

```bash
npm ci
cargo install cargo-llvm-cov --version 0.9.0 --locked
npm run release:gate:macos
```

门禁仅接受 `darwin-arm64`，并固定使用 `runtime.macos.lock.json` 中带 SHA-256 的 Node `22.22.3` 归档。
它依次执行 Windows 配置隔离检查、`git diff --check`、lint、Rust/Node 测试、80% 行覆盖率、主项目与两个
runtime 的零漏洞 audit、两次非缓存资源暂存及内容摘要比较、Darwin 原生模块真实加载、Tauri `.app` 构建、
ad-hoc codesign 深度校验、DMG 校验、隔离 `$DSH_HOME` 的 CoreReady/PluginsReady 与 `--quit-existing`
生命周期冒烟，以及 Host PID 清理。

`.app` 不得超过 600 MiB，DMG 不得超过 300 MiB。测试、示例、源码映射、类型声明、其他平台 PTY
prebuild 和已由插件客户端打包的重复依赖不得进入资源闭包。任何阶段失败时均删除候选发布目录并写失败报告；
只有全部通过后才原子生成 `.release-work/macos/gated`。GitHub 标签 workflow 只允许上传该目录中的 DMG、
`.sha256` 和 `release-gate-macos.json`，不能直接上传 `target` 中的未门禁文件。

## 失败处理

任一功能、回滚、体积、速度或审计门禁失败时，`npm run build` 继续使用 legacy。不得删除失败产物掩盖问题、
放宽阈值或跳过 payload verify。无法执行的门禁必须在发布检查表中保留未完成状态并说明原因。
macOS 任一门禁失败时不得生成或上传 gated DMG；Windows 发布路径和回退策略保持不变。
