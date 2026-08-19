import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { execFileSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { createHash } from "node:crypto";

import { runPlugin } from "../desktop-plugins/runtime-services/lib/index.js";

const repoRoot = resolve(import.meta.dirname, "..");
const runtimeLock = JSON.parse(readFileSync(join(repoRoot, "runtime.lock.json"), "utf8"));
const desktopPackage = JSON.parse(readFileSync(join(repoRoot, "package.json"), "utf8"));
const sourceCommit = execFileSync("git.exe", [
  "-C",
  repoRoot,
  "-c",
  `safe.directory=${repoRoot.replaceAll("\\", "/")}`,
  "rev-parse",
  "HEAD",
], { encoding: "utf8", windowsHide: true }).trim();
if (!/^[0-9a-f]{40}$/.test(sourceCommit)) throw new Error(`invalid source commit: ${sourceCommit}`);
const historicalVersions = ["9.15.9", runtimeLock.pnpm.version, "11.22.0"];
const nodeExecutable = join(repoRoot, "src-tauri", "resources", "node", "node.exe");
const hostRoot = join(repoRoot, "src-tauri", "resources", "host");
const dshEntry = join(hostRoot, runtimeLock.dsh.cliEntry);
const npmCli = process.env.npm_execpath;
const defaultOutput = join(
  repoRoot,
  ".release-work",
  desktopPackage.version,
  "reports",
  "pnpm-compatibility.json",
);

/** 解析唯一可选的 --output 参数，并限制到当前版本发布目录。 */
function parseOutput(arguments_) {
  let output = defaultOutput;
  for (let index = 0; index < arguments_.length; index += 1) {
    if (arguments_[index] !== "--output" || index + 1 >= arguments_.length) {
      throw new Error(`unsupported argument: ${arguments_[index] ?? "<missing>"}`);
    }
    output = resolve(repoRoot, arguments_[index + 1]);
    index += 1;
  }
  const releaseRoot = resolve(repoRoot, ".release-work", desktopPackage.version);
  const relativeOutput = relative(releaseRoot, output);
  if (relativeOutput.startsWith("..") || isAbsolute(relativeOutput)) {
    throw new Error(`compatibility report must stay under ${releaseRoot}`);
  }
  return output;
}

/** 执行外部命令并完整保留 stdout/stderr，失败时给出可定位的命令上下文。 */
function runCommand(command, args, options = {}) {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      windowsHide: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const stdout = [];
    const stderr = [];
    child.stdout.on("data", (chunk) => stdout.push(Buffer.from(chunk)));
    child.stderr.on("data", (chunk) => stderr.push(Buffer.from(chunk)));
    child.once("error", reject);
    child.once("close", (exitCode, signal) => {
      const result = {
        exitCode,
        signal,
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: Buffer.concat(stderr).toString("utf8"),
      };
      if (exitCode !== 0 && !options.allowFailure) {
        reject(new Error(
          `${command} ${args.join(" ")} failed with ${exitCode}:\n${result.stderr}\n${result.stdout}`,
        ));
        return;
      }
      resolvePromise(result);
    });
  });
}

/** 使用当前 Node 直接执行 npm CLI，避免 Windows `.cmd` shell 转义参与测试参数解析。 */
function runNpm(args, options = {}) {
  if (typeof npmCli !== "string" || !isAbsolute(npmCli) || !existsSync(npmCli)) {
    throw new Error("npm_execpath must point to the active npm CLI");
  }
  return runCommand(process.execPath, [npmCli, ...args], options);
}

/** 用精确历史 pnpm 生成真实 modules/lock 元数据；下载只进入测试缓存。 */
async function runHistoricalPnpm(version, profile, args) {
  const npmCache = join(repoRoot, ".runtime-cache", "pnpm-fixtures", "npm");
  await runNpm(
    [
      "exec",
      "--yes",
      `--package=pnpm@${version}`,
      "--cache",
      npmCache,
      "--",
      "pnpm",
      "--dir",
      profile,
      ...args,
    ],
    { cwd: repoRoot },
  );
}

/** 创建声明 DSH bundle 的本地测试包。 */
function createPluginPackage(root, name, version, options = {}) {
  const directory = join(root, `${name}-${version}`);
  mkdirSync(join(directory, "lib"), { recursive: true });
  writeFileSync(
    join(directory, "package.json"),
    `${JSON.stringify({
      name,
      version,
      private: true,
      type: "module",
      main: "lib/index.js",
      scripts: options.scripts ?? {},
      dsh: { bundle: { patch: "./cordis.patch.yml" } },
    }, null, 2)}\n`,
  );
  writeFileSync(join(directory, "lib", "index.js"), "export function apply() {}\n");
  writeFileSync(join(directory, "cordis.patch.yml"), "[]\n");
  for (const [name_, content] of Object.entries(options.files ?? {})) {
    writeFileSync(join(directory, name_), content);
  }
  return directory;
}

/** 将测试包打成真实 npm tarball，并返回绝对路径。 */
async function packPackage(directory, destination) {
  mkdirSync(destination, { recursive: true });
  const result = await runNpm(
    ["pack", "--json", "--pack-destination", destination, "--cache", join(repoRoot, ".runtime-cache", "pnpm-fixtures", "npm")],
    { cwd: directory },
  );
  const metadata = JSON.parse(result.stdout);
  assert.equal(metadata.length, 1);
  return join(destination, metadata[0].filename);
}

/** 初始化带 prepare 构建的本地 Git 依赖并返回固定 commit URL。 */
async function createGitFixture(root) {
  const directory = join(root, "dsh-compat-git");
  mkdirSync(directory, { recursive: true });
  writeFileSync(join(directory, "prepare.cjs"), [
    "const { mkdirSync, writeFileSync } = require('node:fs');",
    "mkdirSync('lib', { recursive: true });",
    "writeFileSync('lib/index.js', 'export function apply() {}\\n');",
    "writeFileSync('prepare.marker', 'prepared\\n');",
    "",
  ].join("\n"));
  writeFileSync(join(directory, "cordis.patch.yml"), "[]\n");
  writeFileSync(join(directory, "package.json"), `${JSON.stringify({
    name: "dsh-compat-git",
    version: "1.0.0",
    private: true,
    type: "module",
    main: "lib/index.js",
    scripts: { prepare: "node prepare.cjs" },
    dsh: { bundle: { patch: "./cordis.patch.yml" } },
  }, null, 2)}\n`);
  await runCommand("git.exe", ["init", "--quiet"], { cwd: directory });
  await runCommand("git.exe", ["add", "."], { cwd: directory });
  await runCommand(
    "git.exe",
    ["-c", "user.name=DSH Fixture", "-c", "user.email=dsh-fixture@example.invalid", "commit", "--quiet", "-m", "fixture"],
    { cwd: directory },
  );
  const commit = (await runCommand("git.exe", ["rev-parse", "HEAD"], { cwd: directory })).stdout.trim();
  return `${pathToFileURL(directory).href.replace(/^file:/, "git+file:")}#${commit}`;
}

/** 对文件或目录生成与时间戳无关的稳定摘要。 */
function snapshotPath(path) {
  if (!existsSync(path)) return null;
  const hash = createHash("sha256");
  const visit = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true }).sort((left, right) => left.name.localeCompare(right.name))) {
      const absolute = join(directory, entry.name);
      const item = relative(path, absolute).replaceAll("\\", "/");
      hash.update(item);
      if (entry.isDirectory()) visit(absolute);
      else if (entry.isFile()) hash.update(readFileSync(absolute));
      else hash.update("unsupported");
    }
  };
  visit(path);
  return hash.digest("hex");
}

/** 捕获事务承诺保护的控制文件原始字节。 */
function snapshotControls(profile, dshHome) {
  const files = [
    join(profile, "package.json"),
    join(profile, "pnpm-lock.yaml"),
    join(profile, "pnpm-workspace.yaml"),
    join(profile, "cordis.patch.yml"),
    join(dshHome, "cordis.patch.yml"),
  ];
  return new Map(files.map((path) => [path, existsSync(path) ? readFileSync(path) : null]));
}

/** 使用桌面 Runtime Services 执行一次真实 DSH plugin 操作。 */
async function runDesktopOperation(args, profile) {
  const stdout = [];
  const stderr = [];
  const handle = runPlugin(args, profile);
  handle.stdout.on("data", (chunk) => stdout.push(Buffer.from(chunk)));
  handle.stderr.on("data", (chunk) => stderr.push(Buffer.from(chunk)));
  const result = await handle.done;
  return {
    ...result,
    stdout: Buffer.concat(stdout).toString("utf8"),
    stderr: Buffer.concat(stderr).toString("utf8"),
  };
}

/** 创建由指定历史 pnpm 写入的 profile。 */
async function createHistoricalProfile(caseRoot, version, fixtures) {
  const dshHome = join(caseRoot, ".dsh");
  const profile = join(dshHome, "profiles", "web");
  const buildDepPath = `file:${relative(profile, fixtures.buildTarball).replaceAll("\\", "/")}`;
  mkdirSync(profile, { recursive: true });
  writeFileSync(join(dshHome, "cordis.patch.yml"), "- id: global-original\n");
  writeFileSync(join(profile, "cordis.patch.yml"), "- id: profile-original\n");
  writeFileSync(join(profile, "pnpm-workspace.yaml"), [
    "packages:",
    "  - .",
    "allowBuilds:",
    `  'dsh-compat-build@${buildDepPath}': true`,
    `  'dsh-compat-git@${fixtures.gitUrl}': true`,
    "",
  ].join("\n"));
  writeFileSync(join(profile, "package.json"), `${JSON.stringify({
    name: "dsh-profile-web",
    private: true,
    packageManager: `pnpm@${version}`,
    dependencies: { "dsh-compat-seed": `file:${fixtures.seed.replaceAll("\\", "/")}` },
    dsh: { profile: { bundles: ["@deepseek-ai/dsh-base", "dsh-compat-seed"] } },
  }, null, 2)}\n`);
  await runHistoricalPnpm(version, profile, [
    "install",
    "--ignore-scripts",
    "--public-hoist-pattern=dsh-legacy-*",
  ]);
  return { dshHome, profile };
}

/** 运行一组历史 profile 的完整兼容与回滚场景。 */
async function runVersionCase(root, version, fixtures) {
  const caseRoot = join(root, `pnpm-${version}`);
  mkdirSync(caseRoot, { recursive: true });
  const { dshHome, profile } = await createHistoricalProfile(caseRoot, version, fixtures);
  const previous = new Map([
    ["DSH_HOME", process.env.DSH_HOME],
    ["DSH_DESKTOP_WEB_PROFILE", process.env.DSH_DESKTOP_WEB_PROFILE],
    ["DSH_DESKTOP_NODE_EXECUTABLE", process.env.DSH_DESKTOP_NODE_EXECUTABLE],
    ["DSH_DESKTOP_CLI_ENTRY", process.env.DSH_DESKTOP_CLI_ENTRY],
    ["DSH_DESKTOP_HOST_ROOT", process.env.DSH_DESKTOP_HOST_ROOT],
  ]);
  const operations = [];
  try {
    process.env.DSH_HOME = dshHome;
    process.env.DSH_DESKTOP_WEB_PROFILE = profile;
    process.env.DSH_DESKTOP_NODE_EXECUTABLE = nodeExecutable;
    process.env.DSH_DESKTOP_CLI_ENTRY = dshEntry;
    process.env.DSH_DESKTOP_HOST_ROOT = hostRoot;

    const invoke = async (name, args, expected = 0) => {
      const started = performance.now();
      const result = await runDesktopOperation(args, profile);
      operations.push({ name, args, exitCode: result.exitCode, durationMs: Math.round(performance.now() - started) });
      if (expected === "nonzero") {
        assert.notEqual(result.exitCode, 0, `${name} unexpectedly succeeded`);
      } else {
        assert.equal(result.exitCode, expected, `${name} failed:\n${result.stderr}\n${result.stdout}`);
      }
      return result;
    };

    await invoke("local-link", ["add", `link:${fixtures.link.replaceAll("\\", "/")}`]);
    await invoke("local-link-remove", ["remove", "dsh-compat-link"]);
    await invoke("tarball-add", ["add", fixtures.tarballV1]);
    await invoke("tarball-update", ["update", "dsh-compat-tarball"]);
    await invoke("tarball-remove", ["remove", "dsh-compat-tarball"]);
    const buildResult = await invoke("allow-builds", ["add", fixtures.buildTarball]);
    assert(
      existsSync(join(profile, "node_modules", "dsh-compat-build", "install.marker")),
      `allowBuilds 未执行授权脚本:\n${buildResult.stderr}\n${buildResult.stdout}`,
    );
    await invoke("allow-builds-remove", ["remove", "dsh-compat-build"]);
    await invoke("git-prepare", ["add", fixtures.gitUrl]);
    assert(existsSync(join(profile, "node_modules", "dsh-compat-git", "lib", "index.js")), "git prepare 未生成入口");
    await invoke("git-remove", ["remove", "dsh-compat-git"]);

    const controls = snapshotControls(profile, dshHome);
    const tree = snapshotPath(join(profile, "node_modules"));
    await invoke("failure-restore", ["add", join(caseRoot, "missing-package.tgz")], "nonzero");
    for (const [path, bytes] of controls) {
      const actual = existsSync(path) ? readFileSync(path) : null;
      assert.deepEqual(actual, bytes, `失败后控制文件发生变化: ${path}`);
    }
    assert.equal(snapshotPath(join(profile, "node_modules")), tree, "失败后依赖树字节摘要发生变化");

    const manifest = JSON.parse(readFileSync(join(profile, "package.json"), "utf8"));
    assert.equal(manifest.dependencies["dsh-compat-seed"] !== undefined, true);
    return { historicalVersion: version, fixedVersion: runtimeLock.pnpm.version, operations };
  } finally {
    for (const [name, value] of previous) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
  }
}

/** 创建所有复用 fixture。 */
async function createFixtures(root) {
  const packages = join(root, "packages");
  const tarballs = join(root, "tarballs");
  mkdirSync(packages, { recursive: true });
  const seed = createPluginPackage(packages, "dsh-compat-seed", "1.0.0");
  const link = createPluginPackage(packages, "dsh-compat-link", "1.0.0");
  const tarballV1Directory = createPluginPackage(packages, "dsh-compat-tarball", "1.0.0");
  const buildDirectory = createPluginPackage(packages, "dsh-compat-build", "1.0.0", {
    scripts: { install: "node install.cjs" },
    files: { "install.cjs": "require('node:fs').writeFileSync('install.marker', 'installed\\n');\n" },
  });
  return {
    seed,
    link,
    tarballV1: await packPackage(tarballV1Directory, tarballs),
    buildTarball: await packPackage(buildDirectory, tarballs),
    gitUrl: await createGitFixture(packages),
  };
}

/** 执行矩阵并写入正式发布报告。 */
async function main() {
  const output = parseOutput(process.argv.slice(2));
  for (const path of [nodeExecutable, dshEntry, join(hostRoot, "toolchains", "pnpm-10", "pnpm.cmd")]) {
    if (!existsSync(path)) throw new Error(`staged runtime entry is missing: ${path}`);
  }
  const stagedPnpm = JSON.parse(readFileSync(join(hostRoot, "node_modules", "pnpm", "package.json"), "utf8"));
  assert.equal(stagedPnpm.version, runtimeLock.pnpm.version, "staged pnpm 与 runtime lock 不一致");

  // Windows 系统临时目录可能返回 8.3 短路径；工作区内的隔离根可保证 pnpm dep path 与 allowBuilds 键一致。
  const temporaryParent = join(repoRoot, ".release-work", ".tmp");
  mkdirSync(temporaryParent, { recursive: true });
  const temporaryRoot = mkdtempSync(join(temporaryParent, "pnpm-compat-"));
  let succeeded = false;
  try {
    const fixtures = await createFixtures(temporaryRoot);
    const cases = [];
    for (const version of historicalVersions) cases.push(await runVersionCase(temporaryRoot, version, fixtures));
    const report = {
      schemaVersion: 2,
      generatedAtUtc: new Date().toISOString(),
      desktopVersion: desktopPackage.version,
      sourceCommit,
      nodeVersion: runtimeLock.node.version,
      fixedPnpmVersion: runtimeLock.pnpm.version,
      cases,
    };
    mkdirSync(dirname(output), { recursive: true });
    writeFileSync(output, `${JSON.stringify(report, null, 2)}\n`);
    succeeded = true;
    console.log(`PNPM COMPATIBILITY OK: ${cases.length} historical profiles, report=${output}`);
  } finally {
    if (succeeded) rmSync(temporaryRoot, { recursive: true, force: true });
    else console.error(`PNPM COMPATIBILITY DIAGNOSTICS RETAINED: ${temporaryRoot}`);
  }
}

await main();
