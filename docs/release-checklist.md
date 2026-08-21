# 预览版发布检查表

每周的版本盘点、选型依据和发布结果统一记录在
[DSH 桌面版每周版本同步](weekly-desktop-release.md)；本检查表只负责候选构建与发布门禁。

## Preview.8-11 验收记录

preview.8 第一轮和 preview.9 第二轮公开 payload 门禁均已通过；preview.9 采用 solid LZMA，最终安装器为
82,934,193 字节。preview.10 已切换默认 payload 并通过 80.94% 核心行覆盖率、三组零漏洞 audit、两次
强制构建逐项同 SHA、6/6 升级矩阵和 20 对 warm P95 门禁。最终安装器为 91,814,862 字节，SHA-256 为
`ee7f9a4613a920a0b76eec5af696a2e0aebee21d856e188da87bcfb734fd52df`；payload P95 为 9,707 ms，低于
12,210 ms 门限。还需一个稳定 preview，才允许删除 legacy 暂存路径。

## 版本与锁定输入

- [ ] 版本号与 `v*` 标签一致。
- [ ] 目标仅为 Windows x64；Node、DSH、Market 与 pnpm 版本和当周版本矩阵、`runtime.lock.json` 一致。
- [ ] `runtime.lock.json` 与 `plugins.lock.json` 均为 schema 2，integrity、归档 SHA-256 和 package lock 匹配。
- [ ] 每个插件的 delivery 入口、资产、runtime/native external 和许可证声明完整。
- [ ] staged 边界不包含 resources、cache、target、日志、本机路径或安装包。

## 静态、测试与审计

- [ ] `npm run validate:icons`、`npm run lint`、`npm test` 通过。
- [ ] 核心 Rust 行覆盖率不低于 80%。
- [ ] `npm run audit:release` 的主项目、`runtime-host`、`plugin-runtime` 三组报告均为 0 total/high/critical。
- [ ] pnpm 9/10/11 真实兼容矩阵确认所有操作最终使用唯一的 `pnpm@10.34.5`。
- [ ] `verify:runtime`、`verify:plugins`、`verify:payload` 通过。
- [ ] `git diff --check` 通过，发布范围中没有 cache、安装目录或 debug artifact。
- [ ] 工作区完全干净，全部正式报告的 `sourceCommit` 与候选 HEAD 一致。
- [ ] 三方许可证与 NOTICE 覆盖 Node、Host、插件及新增 Rust/Node 构建依赖。

## Payload 与预算

- [ ] Tauri payload resources 只有 manifest、Node ZIP、Host ZIP、插件 ZIP 四个文件。
- [ ] Node ZIP 只含 `node.exe` 和许可证；安装器不含 PDB、map、类型、测试或示例源码。
- [ ] 三个 ZIP 展开总大小不超过 300 MiB，NSIS 安装包不超过 100 MiB。
- [ ] 相对 47,418 个 legacy 安装资源文件减少至少 90%。
- [ ] warm cache 重复打包不超过 10 分钟，冷暂存与打包不超过 20 分钟。
- [ ] 同输入强制构建两次，三个 ZIP 与 manifest SHA-256 逐项一致。
- [ ] `runtime-debug-symbols.zip` 独立生成，不参与默认安装器和 payload digest。

## 功能与兼容事务

- [ ] 固定 pnpm 10 完成 add/update/remove、`allowBuilds`、local link、tarball 和 git prepare fixture。
- [ ] pnpm 9/10/11 历史 fixture 只在明确 modules/hoist 不兼容时重建一次、重试一次。
- [ ] 失败后 package、lock、workspace、Cordis patch 与 bundle 状态字节一致，旧依赖树恢复。
- [ ] 运行时不使用 `--force`、全局 pnpm、其他 major 或循环恢复。
- [ ] 官方本地插件绝对 `.ts` 路径、函数/对象/类、`inject`、`ctx.effect()` 清理和 patch 顺序通过。
- [ ] DSH Web、Market、插件锁中的全部内置 bundle、用户插件、PTY、sharp、GenUI、Hindsight、Skills/MCP 和主题通过。
- [ ] 原生多模态模型完成图片上传、缩放、格式转换和复用验证；纯文本模型不显示为具备视觉能力。

## Provision、安装与回滚

- [ ] provision 在 Tauri/single-instance 前执行，只登记 candidate，不直接切换 active。
- [ ] 路径穿越、ADS、设备名、大小写冲突、重复条目、symlink/reparse point、ZIP bomb 和截断 ZIP 均被拒绝。
- [ ] 并发 provision、中断 staging、candidate 失败、状态写入失败和垃圾清理测试通过。
- [ ] clean install、运行中升级、legacy 到 payload、连续 payload 升级与损坏资源矩阵通过。
- [ ] preview.10 设置包迁移、旧设置包已卸载保持卸载，以及新设置包重复协调保持卸载通过。
- [ ] candidate 只有真实 Host core/plugins readiness 与插件事务成功后才晋升 active。
- [ ] 升级失败保持旧 active；新 exe 可运行 active 与 previous 两代 ABI。
- [ ] 卸载始终删除桌面 runtime；仅勾选删除应用数据时删除其余 LocalAppData；永不删除 `~/.dsh`。
- [ ] 安装器 smoke 在专用可丢弃 Windows 用户中运行，隔离 install/runtime/profile，且未污染真实 LocalAppData、HKCU 产品键或 Shell 快捷方式。

## 生命周期与性能

- [ ] fake Host 冒烟覆盖 readiness、单实例、关闭到托盘、显式退出和 Host PID 清理。
- [ ] smoke 记录 `tauri.localhost` 为内部 allow，且没有该 host 的 `external_browser` 记录。
- [ ] CoreReady 后崩溃、插件永不完成和恢复路径最多重试一次。
- [ ] 托盘重启严格执行插件 prepare、Host readiness、commit；失败先 rollback 再恢复一次。
- [ ] 添加工作区时原生目录选择器绑定主窗口，任务栏不出现独立 Node 窗口。
- [ ] 正式 payload 安装器冒烟覆盖 candidate 晋升和插件锁中全部内置 bundle 的 readiness。
- [ ] legacy 与 payload 使用相同 seed profile，预热后交替完成 20 对 warm 启动并记录 3 次 cold。
- [ ] payload 启动 P95 劣化不超过 5% 或 100 ms，两者取较大值。

## 灰度与发布

- [ ] preview.7 安装器只作为固定 SHA-256 的 legacy 基线，不计作 payload preview。
- [ ] preview.8 与 preview.9 均保留 `build:legacy`，并分别完成完整公开 payload 门禁。
- [ ] preview.9 额外完成 preview.8 payload 到 preview.9 payload 的停止/运行中升级。
- [ ] preview.8、preview.9 两轮通过后，preview.10 才把默认 build 切到 payload。
- [ ] 默认切换后再经过一个稳定 preview，才删除 legacy 暂存路径。
- [ ] preview.11 保留 legacy 对照链并使用 preview.10 作为 previous payload。
- [ ] 版本化原子产物目录包含 NSIS、`.sha256`、manifest、构建/审计/基准/升级报告、许可证和 debug symbols。
- [ ] Windows 10 22H2 与 Windows 11 x64 干净环境完成离线首次启动。
- [ ] Release 标为 prerelease，注明社区非官方、Windows x64、内置版本和签名状态。
- [ ] 任一未完成项保持 legacy 发布，不通过放宽验收值推进。
