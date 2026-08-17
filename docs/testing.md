# 测试与质量门禁

## 固定命令

```powershell
npm ci
npm run validate:icons
npm run lint
npm test
npm audit
Push-Location runtime-host; npm audit --omit=dev; Pop-Location
Push-Location plugin-runtime; npm audit --omit=dev; Pop-Location
npm run stage:runtime
npm run stage:plugins
npm run coverage
npm run smoke
npm run smoke:startup
```

`npm run coverage` 需要先安装 `llvm-tools-preview` 和 `cargo-llvm-cov`，并通过 `--fail-under-lines 80` 强制 Host、运行时、生命周期、导航、日志和就绪解析核心模块的行覆盖率不低于 80%。应用装配层 `desktop.rs`、`lib.rs`、`main.rs` 由 Windows 冒烟测试覆盖，不计入核心模块行覆盖率。发布前必须在本机完成两类门禁。

自包含 release 和安装器验证使用：

```powershell
pwsh -NoProfile -File scripts/smoke-test.ps1 `
  -Exe '..\src-tauri\target\release\dsh-desktop.exe' `
  -TimeoutSeconds 180 `
  -UseBundledRuntime
pwsh -NoProfile -File scripts/installer-smoke.ps1 `
  -Installer 'src-tauri\target\release\bundle\nsis\<installer>.exe'
pwsh -NoProfile -File scripts/benchmark-startup.ps1 `
  -Exe '<安装目录>\dsh-desktop.exe'
```

## 覆盖范围

Rust 单元测试覆盖 CoreReady/PluginsReady 两级协议、旧 Host 兼容、Host 原点导航、私有 Node/pnpm PATH 优先级、两阶段超时、快速进程树清理、异常退出码、重复退出、日志脱敏与 `5 MiB × 3` 轮转。插件测试覆盖 9 个 bundle 的固定顺序、连续 prepare 的字节与 mtime 幂等、用户安装优先、禁用保持、Skill 管理、事务提交/回滚和真实 Windows junction。runtime-services 的 Node 测试验证核心服务同时可用后只输出一次 CoreReady。

Windows 冒烟测试使用仓库内的 `scripts/fixtures/fake-host.js`：

1. 启动本次构建的桌面程序并记录唯一 Host PID；
2. 确认就绪日志和窗口导航；
3. 再次启动并确认现有窗口收到聚焦事件，Host 数量仍为一个；
4. 关闭主窗口并确认应用和 Host 继续运行；
5. 通过 `--quit-existing` 进入与托盘“退出”相同的清理路径；
6. 确认记录的 Host PID 已结束。

脚本失败兜底只终止本次记录的桌面 PID 与 Host PID，禁止扫描或批量终止其他 `node.exe`。

`startup-scenarios.ps1` 额外覆盖核心先就绪且插件延迟、CoreReady 后崩溃、插件永不完成三种恢复路径。`benchmark-startup.ps1` 对正式安装版执行 20 次热启动并校验 P95 不超过 8 秒，清理当前 digest 后执行 3 次冷启动并校验每次不超过 20 秒。

发布验证先用 fake Host 检查可注入 supervisor 和侧栏设置 API，再安装 NSIS 包并用内置 Node/DSH/插件重跑同一套生命周期检查。安装器冒烟还验证 digest 插件 store 已预置、首次启动不复制完整插件树，且卸载后缓存和用户 profile 均保留。
