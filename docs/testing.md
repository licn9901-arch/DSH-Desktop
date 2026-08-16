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
```

## 覆盖范围

Rust 单元测试覆盖严格就绪协议、Host 原点导航、私有 Node/pnpm PATH 优先级、Host 重启状态机、启动超时、启动失败、异常退出码、重复退出、日志脱敏与 `5 MiB × 3` 轮转。插件测试覆盖 9 个 bundle 的固定顺序、用户安装优先、禁用保持、旧 Skills/MCP 状态迁移、GenUI Skill 首装/升级/修改/删除、Skin patch 历史坏文件恢复、Hindsight 凭据注入、缺失资源、事务提交/回滚、Better Sidebar revision 设置和真实 Windows junction。桌面设置 bundle 的 Node 测试覆盖 allowlist、受保护项、未知包拒绝、只改 bundles、幂等与并发写，以及 Hindsight 字段保留、凭据不回显、URL/路径校验和连接测试。Host supervisor 通过注入 fake child 和 fake process-tree terminator 验证，不启动真实 DSH，也不使用固定等待时间。

Windows 冒烟测试使用仓库内的 `scripts/fixtures/fake-host.js`：

1. 启动本次构建的桌面程序并记录唯一 Host PID；
2. 确认就绪日志和窗口导航；
3. 再次启动并确认现有窗口收到聚焦事件，Host 数量仍为一个；
4. 关闭主窗口并确认应用和 Host 继续运行；
5. 通过 `--quit-existing` 进入与托盘“退出”相同的清理路径；
6. 确认记录的 Host PID 已结束。

脚本失败兜底只终止本次记录的桌面 PID 与 Host PID，禁止扫描或批量终止其他 `node.exe`。

发布验证先用 fake Host 检查可注入 supervisor 和侧栏设置 API，再安装 NSIS 包并用内置 Node/DSH/插件重跑同一套生命周期检查。卸载检查验证本应用安装目录被移除，且不触碰 DSH 用户会话、profile、插件 marker 与配置。
