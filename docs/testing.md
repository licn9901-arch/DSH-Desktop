# 测试与质量门禁

## 固定命令

```powershell
npm ci
npm run validate:icons
npm run lint
npm test
npm audit
npm run coverage
npm run smoke
```

`npm run coverage` 需要先安装 `llvm-tools-preview` 和 `cargo-llvm-cov`，并通过 `--fail-under-lines 80` 强制核心库行覆盖率不低于 80%。GitHub Actions 会安装并执行该门禁。

## 覆盖范围

Rust 单元测试覆盖严格就绪协议、Host 原点导航、运行时路径优先级、启动超时、启动失败、异常退出码、重复退出、日志脱敏与 `5 MiB × 3` 轮转。Host supervisor 通过注入 fake child 和 fake process-tree terminator 验证，不启动真实 DSH，也不使用固定等待时间。

Windows 冒烟测试使用仓库内的 `scripts/fixtures/fake-host.js`：

1. 启动本次构建的桌面程序并记录唯一 Host PID；
2. 确认就绪日志和窗口导航；
3. 再次启动并确认现有窗口收到聚焦事件，Host 数量仍为一个；
4. 关闭主窗口并确认应用和 Host 继续运行；
5. 通过 `--quit-existing` 进入与托盘“退出”相同的清理路径；
6. 确认记录的 Host PID 已结束。

脚本失败兜底只终止本次记录的桌面 PID 与 Host PID，禁止扫描或批量终止其他 `node.exe`。
