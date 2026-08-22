# macOS 单架构打包与发布门禁

本文指导 macOS 打包负责人产出 DeepSeek Harness Desktop 的可发布 DMG，并沿用现有 payload
打包策略和发布门禁。本文是操作手册，不代表当前 `main` 已支持 macOS；当前主分支仍是 Windows x64
实现，不能直接在 Mac 上执行 `npm run build` 后发布。

## 先给结论

1. 预览版默认只发布 Apple Silicon 的 `arm64` DMG。Intel 版本有明确需求时单独构建、单独命名、
   单独验收；不得用 `universal-apple-darwin` 代替两个单架构产物。
2. 必须使用 payload 链路。DMG 内只交付 payload manifest、Node ZIP、Host ZIP、插件 ZIP 四个
   payload 资源，不能把原始 `node_modules` 作为 legacy resources 再打一次。
3. 单架构 DMG 必须不超过 100 MiB，三个 payload ZIP 合计不超过 90 MiB，展开后不超过
   300 MiB 和 20,000 个文件。门禁失败时停止发布，不提高阈值。
4. 正式 DMG 必须在 macOS 上构建。先签署 payload 内的 Node 和全部 Mach-O/native addon，再签署
   外层 App，最后完成 Apple 公证和 stapling。ad-hoc 签名只允许本地开发验证。
5. 包体比较必须使用 DMG 文件的精确字节数。不能拿 Finder 中展开后的 `.app` 大小与 Windows
   的压缩 NSIS 安装器比较。

官方 Tauri 文档确认：DMG 需要在 Mac 上构建；`universal-apple-darwin` 同时包含 Apple Silicon
和 Intel；站外分发需要签名和公证。参考 [Tauri DMG](https://v2.tauri.app/distribute/dmg/)、
[Tauri macOS 签名与公证](https://v2.tauri.app/distribute/sign/macos/) 和
[Tauri App Store 构建目标说明](https://v2.tauri.app/distribute/app-store/)。Apple 的公证要求包括
为交付的可执行代码启用有效签名、Hardened Runtime 和安全时间戳，见
[Apple 公证准备要求](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)。

## 为什么包会接近两倍

按以下顺序检查，不要先猜压缩参数。

| 现象 | 判断方法 | 处理 |
|---|---|---|
| 产物是 universal | `lipo -archs <主程序>` 同时输出 `arm64 x86_64` | 改为 `aarch64-apple-darwin`；Intel 另出一包 |
| 比较的是 `.app` | 路径以 `.app` 结尾，或使用 Finder“显示简介” | 改为比较最终 `.dmg` 的精确字节数 |
| 使用了 legacy resources | `.app` 内同时出现原始 `node_modules` 和三个 payload ZIP | 只保留四个 payload 资源 |
| native 包含双架构或全平台文件 | 存在 `darwin-x64`、`win32-*`、`linux-*` prebuild | 只保留当前目标架构，并重新跑真实 loader smoke |
| 调试文件进入正式包 | 存在 `.dSYM`、`.map`、TypeScript 声明或测试/示例 | 调试产物独立归档，不进入 DMG |
| 同一运行时被复制两次 | Node、Host 或插件同时出现在多个 Resources 子目录 | 以 payload manifest 为唯一安装输入 |
| 旧缓存污染构建 | clean checkout 与日常工作目录产物大小不同 | 在干净 worktree/checkout 强制构建并核对 cache key |

先对现有产物做一次快速诊断：

```bash
DMG="path/to/DeepSeek.Harness.Desktop_VERSION_aarch64.dmg"
APP="path/to/DeepSeek Harness Desktop.app"
BIN="$APP/Contents/MacOS/dsh-desktop"

# DMG 精确大小；发布门禁使用这个值，不使用 du 的磁盘占用值。
stat -f '%z bytes' "$DMG"

# 主程序只能包含目标架构。arm64 正式包不应输出 x86_64。
file "$BIN"
lipo -archs "$BIN"

# 找出体积最大的文件和目录。
du -ak "$APP" | sort -nr | head -40
find "$APP" -type f -size +5M -print

# 检查 native addon 和误带的平台目录。
find "$APP" -type f -name '*.node' -exec file {} \;
find "$APP" -type d \( -name 'win32-*' -o -name 'linux-*' -o -name 'darwin-x64' \) -print

# payload manifest 必须只有一份，三个 ZIP 也各只有一份。
find "$APP" -name 'payload-manifest.json' -o -name 'node-runtime.zip' \
  -o -name 'host-runtime.zip' -o -name 'builtin-plugins.zip'
```

若主程序是单架构，但 DMG 仍接近 Windows 安装器两倍，优先检查 payload 内的 `node-pty`、
`sharp`、`lightningcss`、`@vscode/ripgrep` 及其他 optional native packages。只看顶层依赖数量
没有意义，要看实际进入三个 ZIP 的文件。

## 当前主分支的适配前置

macOS 分支只有满足以下条件后，才可以进入本文后续打包步骤。任一项仍使用 Windows 固定值时，
只能算移植开发包，不能进入发布门禁。

- `runtime.lock.json` 按目标锁定 Node `22.22.3` 的官方 macOS 归档、URL 和 SHA-256；不得覆盖或
  模糊复用 Windows `win-x64` 记录。
- Node payload 入口在 macOS 为 `node/node`，不是 `node/node.exe`；只复制 Node 可执行文件和
  官方许可证，不复制完整 Node 发行目录。
- runtime 根使用 macOS Application Support 目录，不依赖 `LOCALAPPDATA`。
- payload cache key 明确包含 `aarch64-apple-darwin` 或 `x86_64-apple-darwin`、Node/npm/esbuild/Rust
  版本以及全部 lockfile、暂存脚本和 payload 实现摘要。不同平台和架构不得共用缓存。
- `node-pty` 只保留当前 `darwin-*` prebuild；`sharp`、`lightningcss`、ripgrep 及其他 native
  optional dependency 只保留当前架构。裁剪后必须用内置 Node 执行真实加载验证。
- macOS payload 在生成正式 ZIP 前递归识别所有 Mach-O，使用本次 release 的 Developer ID identity
  签名并逐项验证。外层 Tauri 签名无法替代 ZIP 内部代码签名；不得靠关闭 library validation 接受
  未签名 addon。
- 插件托管链接使用 macOS directory symlink，并保持现有的不可变 runtime、candidate/active/previous
  和失败回滚语义。
- Tauri 使用单独的 macOS payload 配置，bundle targets 为 `app` 和 `dmg`，提供 `.icns` 图标、
  `minimumSystemVersion`、签名 identity 和所需 entitlements。不得复用 NSIS hooks。
- Host 进程树退出、单实例、托盘、外部链接、日志路径、卸载/升级和用户数据保留均有 macOS 实现与测试。
- 构建脚本暴露目标参数，不从构建机 CPU 架构静默推断发布目标。

当前主分支中以下文件仍明确写死 Windows，macOS 分支至少要逐项核对：

```text
runtime.lock.json
scripts/stage-runtime.ps1
scripts/stage-payload.ps1
scripts/verify-runtime.ps1
scripts/verify-plugins.ps1
scripts/build-payload.ps1
scripts/release-gate.ps1
scripts/write-release-artifacts.ps1
src-tauri/tauri.conf.json
src-tauri/tauri.payload.conf.json
src-tauri/src/runtime.rs
src-tauri/src/logger.rs
```

不要通过手工改 ZIP、手工删 `.app` 内文件或在签名后用 `lipo -thin` 绕过这些适配。任何签名后的
内容修改都会使签名失效，也会让构建结果脱离 lock 和 payload digest。

## 构建环境

建议使用专用、可丢弃的 macOS 构建用户，避免真实 `~/.dsh`、登录项、Keychain 状态和旧版本应用
干扰门禁。正式构建至少固定：

- Apple Silicon Mac；Intel 包使用独立 target 构建；
- 仓库要求的 Xcode 和 Command Line Tools；
- Node.js `22.22.3`；
- Rust `1.94.1` 与目标 `aarch64-apple-darwin`；
- 仓库 lockfile 固定的 npm、Tauri CLI、DSH、Market、pnpm 和插件版本；
- Developer ID Application 证书及 App Store Connect API 凭据。

检查环境：

```bash
set -euo pipefail

uname -m
sw_vers
xcodebuild -version
node --version
npm --version
rustc --version
rustc --print host-tuple
cargo --version
security find-identity -v -p codesigning
```

预期 `uname -m` 为 `arm64`，Node 为 `v22.22.3`，Rust 为仓库固定版本。构建日志只能记录证书
identity 和 Team ID，禁止输出 API 私钥、Apple ID app-specific password 或任何 token。

## 单架构 payload 构建

### 1. 固定输入并确认工作区

从目标 release commit 创建干净 checkout，记录 commit 和 submodule 状态。发布负责人应先查看
差异边界，不能把 `target`、resources、cache、日志或本机绝对路径提交进仓库。

```bash
git status --short
git rev-parse HEAD
git diff --check
npm ci
```

### 2. 执行静态、测试和审计门禁

macOS 分支应提供平台无关命令，至少执行：

```bash
npm run validate:icons
npm run lint
npm test
npm run coverage
npm run audit:release
npm run test:pnpm-compat
```

要求核心 Rust 行覆盖率不低于 80%，主项目、`runtime-host`、`plugin-runtime` 三组 audit 均为
0 total/high/critical。`pnpm@10.34.5` 兼容事务仍需覆盖 add/update/remove、单次重建、单次重试和
失败恢复。

### 3. 暂存并裁剪目标 runtime

macOS 分支应把目标作为显式参数。推荐统一对外命令如下；如果分支采用其他脚本名，发布说明必须
给出一一对应关系，不能回退到手工复制：

```bash
TARGET=aarch64-apple-darwin

npm run stage:runtime -- --target "$TARGET"
npm run verify:runtime -- --target "$TARGET"
npm run stage:plugins -- --target "$TARGET"
npm run verify:plugins -- --target "$TARGET"
npm run verify:payload-reproducibility:mac -- --target "$TARGET"
npm run package:payload:mac -- --target "$TARGET" --force
npm run verify:payload -- --target "$TARGET"
```

说明：当前 `main` 还不接受这些 macOS 参数；命令是 macOS 分支必须实现的发布接口。若执行时参数
被忽略、仍查找 `node.exe`/`win32-x64`，立即停止，不要继续调用 Tauri。

裁剪后必须使用 payload 内置 Node，而不是系统 Node，至少真实加载：

```bash
"<payload-node>" -e "require('yaml'); require('@opentelemetry/api'); require('@aws-sdk/client-bedrock-runtime'); require('node-pty'); require('sharp'); console.log('runtime-loader-ok')"
```

### 4. 检查 payload 预算

`src-tauri/resources/payload` 必须且只能包含下列四个文件：

```text
payload-manifest.json
node-runtime.zip
host-runtime.zip
builtin-plugins.zip
```

使用 Node 读取结构化 manifest，不用 ZIP 文件的表面大小代替 manifest 的展开统计：

```bash
node - <<'NODE'
const fs = require('node:fs');
const p = JSON.parse(fs.readFileSync('src-tauri/resources/payload/payload-manifest.json', 'utf8'));
const parts = [p.nodeRuntime, p.hostRuntime, p.builtinPlugins];
const total = key => parts.reduce((sum, item) => sum + Number(item[key]), 0);
const result = {
  desktopVersion: p.desktopVersion,
  payloadDigest: p.payloadDigest,
  entries: p.entries,
  files: total('fileCount'),
  compressedBytes: total('compressedSize'),
  unpackedBytes: total('unpackedSize'),
};
console.log(JSON.stringify(result, null, 2));
if (result.files > 20_000) process.exitCode = 1;
if (result.compressedBytes > 90 * 1024 * 1024) process.exitCode = 1;
if (result.unpackedBytes > 300 * 1024 * 1024) process.exitCode = 1;
NODE

test "$(find src-tauri/resources/payload -type f | wc -l | tr -d ' ')" = "4"
```

入口必须是 `node/node`、锁定的 DSH `lib/bin.js` 和 `plugins/node_modules`。默认包不能包含
`.dSYM`、PDB、source map、TypeScript 源码或声明、测试和示例；许可证、NOTICE 和运行时需要的
`SKILL.md` 必须保留。不要按目录名盲删 `doc/docs`，因为依赖可能把运行时代码放在这些目录中。

### 5. 验证未签名闭包可复现

Developer ID 的安全时间戳会改变签名后的 Mach-O 字节，因此 macOS 可复现性门禁分成两层：

1. 不使用签名凭据，清空本次 target cache 或使用 `--force`，对裁剪完成的未签名 runtime 闭包构建
   两次。逐项比较规范化文件清单、文件 SHA-256，以及三个临时 ZIP 和 manifest；结果必须完全一致。
2. 可复现性通过后重新创建干净 staging，进入正式签名和打包。签名后的 payload 与最终 DMG 记录
   SHA-256，但不要求两次签名产物逐字节相同。

未签名临时 ZIP 的摘要可使用：

```bash
shasum -a 256 src-tauri/resources/payload/payload-manifest.json \
  src-tauri/resources/payload/node-runtime.zip \
  src-tauri/resources/payload/host-runtime.zip \
  src-tauri/resources/payload/builtin-plugins.zip
```

未签名 ZIP 只能作为构建中间件，不得上传或交给用户。可复现性报告必须同时记录 target triple、
未签名闭包摘要、两次对比结果和最终签名 payload digest，防止拿不同输入做对比。

### 6. 签署 payload 内部代码

进入本阶段前，从 Keychain 或 CI secret store 注入后文所列的签名 identity；不要在命令行中直接输入
真实凭据。正式 `package:payload:mac` 必须在压缩 ZIP 前完成以下阶段：

1. 在 staging 中使用 `file` 或 Mach-O parser 找出 Node、全部 `.node` addon 和其他 Mach-O 文件。
2. 对每个文件使用同一 `Developer ID Application` identity、Hardened Runtime 和安全时间戳签名。
3. 对每个文件执行 `codesign --verify --strict --verbose=2`，并校验实际 CPU 架构等于目标架构。
4. 运行内置 Node loader smoke，确认签名没有破坏 `node-pty`、`sharp` 等 native addon。
5. 生成三个正式 ZIP 和 manifest，再使用 Rust `payload-tool` 验证摘要、入口、文件数和展开上限。

外层 App 签名前还要将正式 payload 展开到临时目录，递归复验全部 Mach-O 签名。只执行
`codesign --verify --deep <App>` 看不到 ZIP 内部，因此不能替代这项检查。临时目录必须由构建脚本创建、
限定在构建根内并在验证后清理。

### 7. 构建 arm64 App 和 DMG

Tauri 官方目标名为 `aarch64-apple-darwin`。macOS 分支应提供等价的 payload build 命令，内部先
`--no-bundle` 构建，再以同一 target 打包 `app,dmg`：

```bash
TARGET=aarch64-apple-darwin

npm run build:payload:mac -- --target "$TARGET"
```

其内部等价阶段应是：

```bash
npm run tauri -- build --no-bundle --target "$TARGET" --config src-tauri/tauri.macos.payload.conf.json
npm run tauri -- bundle --bundles app,dmg --target "$TARGET" --config src-tauri/tauri.macos.payload.conf.json
```

不要添加 `--target universal-apple-darwin`。如果确实发布 Intel 版本，在独立 clean build 中改为
`x86_64-apple-darwin`，并重新执行暂存、native loader、payload、签名、公证和全部门禁。

## 签名与公证

正式 DMG 使用 `Developer ID Application` identity。推荐让 Tauri 读取 CI/本机临时环境变量完成签名
与公证；App Store Connect API 方式使用：

```bash
export APPLE_SIGNING_IDENTITY='Developer ID Application: <组织名> (<TEAM_ID>)'
export APPLE_API_ISSUER='<issuer-id>'
export APPLE_API_KEY='<key-id>'
export APPLE_API_KEY_PATH='<绝对路径>/AuthKey_<key-id>.p8'
```

这些值只能来自 Keychain 或 secret store，不写入 shell history、仓库、构建报告和 Release 说明。
Tauri 也支持 Apple ID app-specific password，但团队发布优先使用职责清晰、可撤销的 API key。

构建后验证 App、DMG 和 stapled ticket：

```bash
APP="src-tauri/target/aarch64-apple-darwin/release/bundle/macos/DeepSeek Harness Desktop.app"
DMG="$(find src-tauri/target/aarch64-apple-darwin/release/bundle/dmg -maxdepth 1 -name '*.dmg' -print -quit)"

codesign --verify --deep --strict --verbose=2 "$APP"
codesign -dv --verbose=4 "$APP" 2>&1 | sed -n '1,20p'
spctl --assess --type execute --verbose=4 "$APP"
xcrun stapler validate "$APP"
xcrun stapler validate "$DMG"
shasum -a 256 "$DMG"
stat -f '%z' "$DMG"
```

`codesign --deep` 这里只用于验证，不用于事后补签。Node 可执行文件和所有 `.node` native addon 必须
在生成 ZIP 前由 payload 内层签名阶段处理并复验；不得在公证后修改 payload 或可执行文件。

## macOS 发布门禁

### 体积与结构

- [ ] 正式目标是 `aarch64-apple-darwin`，主程序和所有 Mach-O/native addon 均为 arm64。
- [ ] DMG 精确大小不超过 104,857,600 字节（100 MiB）。
- [ ] payload resources 恰好四个文件，三个 ZIP 合计不超过 94,371,840 字节（90 MiB）。
- [ ] payload 展开后不超过 314,572,800 字节（300 MiB）和 20,000 个文件。
- [ ] 不包含其他平台 prebuild、原始 legacy runtime、重复 Node/Host/插件树或构建 cache。
- [ ] `.dSYM`、source map 等调试产物独立归档，不进入 DMG 和 payload digest。
- [ ] 同输入两次未签名闭包的文件清单、文件 SHA-256、三个临时 ZIP 与 manifest 逐项一致。
- [ ] 最终三个 ZIP 内全部 Mach-O 已签名、架构正确，展开复验通过并记录正式 payload digest。

### 功能、升级与性能

- [ ] 干净 macOS 用户从 DMG 拖入 `/Applications`，断网首次启动成功。
- [ ] candidate 经真实 Host core/plugins readiness 后才晋升 active，失败继续使用旧 active。
- [ ] DSH Web、Market、全部内置 bundle、用户插件、PTY、sharp、GenUI、Hindsight、ModLens、
  Skills/MCP 和主题完成真实验证。
- [ ] 单实例、关闭到托盘、托盘重启、显式退出、Host 进程树清理和外部链接行为正确。
- [ ] 上一 macOS 预览版到当前版的停止/运行中升级、损坏 payload、失败回滚与连续升级通过。
- [ ] 卸载应用不删除 `~/.dsh`；桌面托管 runtime 和其余应用数据按 macOS 卸载说明处理。
- [ ] 使用相同 seed profile 交替执行 20 对 warm 启动并记录 3 次 cold；新 payload P95 劣化不超过
  5% 或 100 ms，两者取较大值。
- [ ] Apple Silicon 最低支持版本的干净虚拟机或实体机完成 Gatekeeper 首次启动验证。

### 安全与发布材料

- [ ] 主项目、runtime-host、plugin-runtime 三组 audit 为 0 total/high/critical。
- [ ] App 和 DMG 的签名、公证、stapling、`spctl` 检查全部通过。
- [ ] 签名采用 Hardened Runtime 和安全时间戳；ZIP 内部代码的签名已独立验证。
- [ ] entitlement 只包含实际需要的能力，没有为解决签名问题临时扩大权限。
- [ ] 发布目录原子生成，文件名明确包含版本和 `aarch64`，不会覆盖 Intel 或 Windows 产物。
- [ ] Release 标为 prerelease，并说明社区非官方、支持的 macOS/CPU、内置版本和签名状态。

任何一项未完成都不能把产物称为“已通过门禁”。无法执行的项目必须保留为未完成并记录原因，
不能写成“人工确认”后跳过。

## 发布产物

arm64 产物建议命名：

```text
DeepSeek.Harness.Desktop_<version>_aarch64.dmg
DeepSeek.Harness.Desktop_<version>_aarch64.dmg.sha256
```

版本化发布目录至少包含：

```text
<DMG>
<DMG>.sha256
payload-manifest.json
payload-build-report.json
npm-audit.json
payload-reproducibility.json
startup-comparison.json
upgrade-matrix.json
release-gate.json
release-gate.md
runtime-debug-symbols.zip
third-party-licenses.json
plugin-third-party-licenses.json
THIRD_PARTY_NOTICES.md
build-summary.md
```

`build-summary.md` 必须记录 release commit、目标 triple、macOS/Xcode/Node/Rust/Tauri 版本、payload
digest、四个 payload SHA-256、DMG 精确字节数、DMG SHA-256、签名 identity、Team ID、公证结果、
最低系统版本和全部门禁报告路径。不得记录任何 secret。

## 发给复核人的最小证据

打包负责人提交以下结果后，复核人才开始发布检查：

1. `git rev-parse HEAD` 和干净的 `git status --short`。
2. `uname -m`、`rustc --print host-tuple`、Node/Rust/Xcode 版本。
3. payload manifest 汇总：文件数、压缩/展开字节数和 payload digest。
4. 两次未签名 payload 闭包的文件清单、文件 SHA-256 和四项中间件摘要对比。
5. DMG 精确字节数、SHA-256，以及主程序和全部 `.node` 的 `file`/`lipo` 结果。
6. `codesign`、`spctl`、stapler 验证结果。
7. audit、功能、升级、性能和 release gate 报告。
8. `du -ak <App> | sort -nr | head -40` 的体积 Top 40，便于发现回归。

如果当前包仍接近目标的两倍，先提供上述第 3、5、8 项。它们足以区分 universal、重复 resources、
跨平台 native 依赖和“拿展开 App 与压缩 DMG 比较”这四类问题。
