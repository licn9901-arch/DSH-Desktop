# DeepSeek Harness Desktop（Tauri 版）

把 DeepSeek Harness 的 Web GUI 封装成 Windows 桌面应用（方案 B：Tauri v2 + WebView2）。

## 原理

DSH 的界面本质是"本地 HTTP 服务 + 浏览器页面"：`dsh web` 在回环地址上起一个服务，
页面由 host 注入 `window.__DSH_BOOT__` 并提供 HTTP/WS 通道，无法静态打包成纯前端。
所以所有桌面化方案都是**壳 + 内部启动 dsh web 进程**——本工程的 Rust 主逻辑与官方
Electron 壳（`C:\Program Files\DeepSeek Harness\resources\app.asar` 里的
`host-supervisor`）完全同构：

1. `spawn(node --expose-internals <dsh>/lib/bin.js web --host 127.0.0.1 --port 0)`
   （`--port 0` = 随机端口，避免冲突；主程序 debug/release 均为 GUI subsystem，
   spawn 时带 `CREATE_NO_WINDOW`，node 子进程不会闪黑框）
2. 逐行读 host stdout，等一行 `dsh web: http://127.0.0.1:<端口>` 即为就绪
   （URL 校验规则照抄官方壳：http、回环主机、显式端口、路径为 `/`）
3. 打开 WebView2 窗口加载该 URL（启动期间先显示 `ui/index.html` 占位页）
4. 窗口关闭 / 异常退出时，先 `taskkill /T` 温和关闭进程树，1.2s 后强杀兜底
   （`taskkill` 同样隐藏控制台窗口）
5. host 的 stdout/stderr 全部追加到 `%LOCALAPPDATA%\dsh-desktop\dsh-desktop.log`

## 目录结构

```
dsh-tauri/
├── build.cmd            # 一键构建（检查工具链 → 生成黑色鲸鱼图标 → npm install → tauri build）
├── dev.cmd              # 一键调试构建并运行
├── uninstall.cmd        # 从注册表找到 NSIS 生成的卸载器并启动
├── ui/index.html        # 启动占位页（embedded into the binary）
├── scripts/generate-icons.ps1   # 用 Edge headless 把 whale.svg 精确渲染成 icon.png / 多尺寸 icon.ico
├── scripts/smoke-test.ps1       # 冒烟测试：启动 exe → 等日志出现 host ready → 清理进程树
└── src-tauri/
    ├── tauri.conf.json  # v2 配置：无默认窗口（窗口由 Rust 代码创建）、NSIS 安装/卸载
    ├── capabilities/default.json
    ├── icons/whale.svg  # DeepSeek Harness 官方黑色鲸鱼 SVG 源图
    ├── nsis/installer.nsh  # NSIS 钩子：额外创建开始菜单卸载快捷方式
    └── src/main.rs      # 全部逻辑：spawn host、无黑窗启动、就绪解析、窗口、退出清理
```

## 环境要求（首次一次性）

| 依赖 | 说明 |
|---|---|
| Node.js ≥ 18 | 已有（`C:\nvm4w\nodejs`） |
| Rust 工具链 | `https://rustup.rs` 装 rustup-init（默认选项） |
| VS Build Tools（MSVC） | "Desktop development with C++" 工作负载，`https://visualstudio.microsoft.com/visual-cpp-build-tools/` |
| WebView2 运行时 | Win10/11 自带（Edge 同源） |
| Microsoft Edge | 图标生成使用 Edge headless 渲染 SVG（Win10/11 自带） |
| 网络 | 首次构建从 crates.io 拉取 ~400 个 crate（10–30 分钟） |

## 构建与运行

```bat
build.cmd     :: 首次构建 + 打 NSIS 安装包（src-tauri\target\release\bundle\nsis\*.exe）
dev.cmd       :: 调试构建并直接运行
```

构建完成后的可执行文件：`src-tauri\target\debug\dsh-desktop.exe`（debug）/
`src-tauri\target\release\dsh-desktop.exe`（release）。两者都已是 Windows GUI 子系统，
启动时不会出现黑色控制台窗口；但只有 NSIS 安装包会注册卸载信息。

## 安装与卸载

正式使用请安装 NSIS 安装包，而不是直接双击 debug/release exe：

```bat
src-tauri\target\release\bundle\nsis\DeepSeek Harness Desktop_0.1.0_x64-setup.exe
```

安装器默认装到 `%LOCALAPPDATA%\DeepSeek Harness Desktop`，并会：

- 注册到“设置 → 应用 → 已安装的应用”，可像普通 Windows 软件一样卸载；
- 在安装目录写入 `uninstall.exe`；
- 在开始菜单创建应用快捷方式和 `Uninstall DeepSeek Harness Desktop` 卸载快捷方式；
- 卸载时勾选“删除应用数据”会一并清理 `%LOCALAPPDATA%\dsh-desktop` 日志目录。

卸载方式任选其一（建议先关闭应用窗口；卸载器若检测到应用仍在运行，也会提示结束进程）：

1. Windows 设置 → 应用 → 已安装的应用 → DeepSeek Harness Desktop → 卸载；
2. 开始菜单文件夹“DeepSeek Harness Desktop”里的 `Uninstall DeepSeek Harness Desktop`；
3. 运行仓库根目录的 `uninstall.cmd`。

## 配置（环境变量，均可选）

| 变量 | 作用 |
|---|---|
| `DSH_DESKTOP_NODE_EXECUTABLE` | 指定 node 可执行文件（默认：PATH 里的 node.exe） |
| `DSH_DESKTOP_CLI_ENTRY` | 指定 dsh CLI 入口 `lib/bin.js`（默认自动探测） |
| `DSH_DESKTOP_CWD` | host 进程工作目录（默认 `%USERPROFILE%`，与你现有 `start-dsh-web.cmd` 的 `cd /d "%USERPROFILE%"` 一致） |
| `DSH_DESKTOP_READY_TIMEOUT_SECS` | 等待 host 就绪 URL 的超时秒数（默认 90，只影响就绪前的等待） |

CLI 入口自动探测顺序：
1. `DSH_DESKTOP_CLI_ENTRY`
2. 应用同目录 `resources/host/...`（自包含分发时使用，见下）
3. `C:\Program Files\DeepSeek Harness\resources\host\...`（官方桌面安装的 host，rc.5，与官方壳同款配对）
4. 全局 npm 安装的 `dsh`（`<node目录>\node_modules\@deepseek-ai\dsh\lib\bin.js`，当前 rc.6）

## 启动慢 / 超时排查

日志现在会记录实际就绪耗时：`[app] host ready: http://127.0.0.1:<端口> (started in ... ms)`。

- 只要日志里已经出现 `host ready`，就说明 DSH 本身启动成功了。之前报
  `Timed out waiting...` 是壳逻辑在就绪后仍继续 90 秒计时导致的，已修复；
  现在导航到 host URL 后不会再触发该超时。
- 如果 `host ready` 之前的耗时确实很长，可尝试把 `DSH_DESKTOP_CWD` 指向一个较小的实际工作目录，
  避免默认在 `%USERPROFILE%` 下初始化。
- 也可以显式指定全局 npm 的 dsh（rc.6）入口：
  `DSH_DESKTOP_CLI_ENTRY=C:\nvm4w\nodejs\node_modules\@deepseek-ai\dsh\lib\bin.js`
- 首次启动通常最慢（Node/DSH 冷启动）；同一登录会话内再次启动会快不少。

## 做成自包含分发（可选，发布给别人用）

当前运行依赖系统 node + 已安装的 dsh（或官方 Program Files 里的 host）。要做一个
不依赖环境的安装包：

1. 把官方 host 运行时复制进来：`robocopy "C:\Program Files\DeepSeek Harness\resources\host" src-tauri\resources\host /E`
   （约数百 MB，含全部 `@deepseek-ai/dsh-*` 包和前端 dist）
2. 复制一个 node：`copy C:\nvm4w\nodejs\node.exe src-tauri\resources\node\node.exe`
3. 在 `tauri.conf.json` 的 `bundle` 里加：
   ```json
   "resources": ["resources/host", "resources/node/node.exe"]
   ```
4. 重新 `build.cmd`

主逻辑已经按这个布局写好探测逻辑（`resolve_cli_entry` / `resolve_node` 的顺序里 bundled
优先）。

## 已知边界（MVP 未做，后续可加）

- 无托盘图标 / 无最小化到托盘（官方 Electron 版有）
- 无单实例锁（开两个会起两个 host；host 数据按会话目录隔离，风险低）
- 无开机自启、无自动更新
- host 在就绪后意外崩溃会直接退出应用（MVP 行为）
- WebView2 的 Web Notifications 支持有限（DSH 的"任务完成"等提示走页面内通知，不受影响）
- 仅 Windows（Tauri 跨平台，但本工程未配 mac/Linux 的 host 路径与打包目标）

## 与官方 Electron 版的工作量对照

| 内容 | 官方 Electron 壳 | 本工程（Tauri） |
|---|---|---|
| 壳逻辑（spawn/就绪/清理） | host-supervisor.js | `src/main.rs`（同协议） |
| 体积 | 安装包 ~数百 MB（含 Chromium） | 安装包 ~10–15 MB + host 运行时（自包含时数百 MB） |
| 内存 | Chromium 整包 | 复用系统 WebView2（Edge），更省 |
| 首次构建 | npm 装依赖 | 额外编译 ~400 个 Rust crate（10–30 分钟） |

## 故障排查

- 窗口停在"正在启动"：看 `%LOCALAPPDATA%\dsh-desktop\dsh-desktop.log` 的 `[host]` 行
- `tauri build` 报 linker 错误：MSVC C++ 工具链缺失，装 VS Build Tools
- 启动即报"could not locate the dsh CLI entry"：设置 `DSH_DESKTOP_CLI_ENTRY` 指向
  `<任意 dsh 安装>/node_modules/@deepseek-ai/dsh/lib/bin.js`
- 打包时 `Downloading https://github.com/.../nsis-3.11.zip` 卡住或 `timeout: global`：
  GitHub 直连不通时，用镜像手动下载并放进 tauri 缓存：
  ```powershell
  $nsisRoot = Join-Path $env:LOCALAPPDATA 'tauri\NSIS'
  New-Item -ItemType Directory -Force -Path $nsisRoot | Out-Null
  Invoke-WebRequest -Uri "https://ghfast.top/https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.11/nsis-3.11.zip" `
    -OutFile (Join-Path $nsisRoot 'nsis-3.11.zip') -TimeoutSec 90
  Expand-Archive -Path (Join-Path $nsisRoot 'nsis-3.11.zip') -DestinationPath (Join-Path $nsisRoot 'nsis-3.11') -Force
  ```
  然后重跑 `npx tauri build`（下载成功时会自动重建缓存并校验哈希）。
