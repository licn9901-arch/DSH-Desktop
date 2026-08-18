import assert from "node:assert/strict";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { test } from "node:test";
import { delimiter, join } from "node:path";
import {
  apply,
  captureControlFiles,
  isPnpmCompatibilityFailure,
  packageManagerEnvironment,
  readProfileSnapshot,
  reconcileProfileBundles,
  restoreControlFiles,
  runPlugin,
} from "../lib/index.js";

test("核心服务就绪后只输出一次桌面 CoreReady 协议", () => {
  const previousProfile = process.env.DSH_DESKTOP_WEB_PROFILE;
  const messages = [];
  const originalLog = console.log;
  process.env.DSH_DESKTOP_WEB_PROFILE = join(tmpdir(), "dsh-runtime-core-ready");
  console.log = (message) => messages.push(message);
  try {
    const provided = new Map();
    const ctx = {
      webServer: { host: "127.0.0.1", port: 4321 },
      webRuntime: {},
      provide(name, value) { provided.set(name, value); },
    };
    apply(ctx);
    apply(ctx);
    assert.deepEqual(messages, ["dsh desktop-core: http://127.0.0.1:4321"]);
    assert.ok(provided.has("desktopProfiles"));
    assert.ok(provided.has("desktopPnpm"));
  } finally {
    console.log = originalLog;
    if (previousProfile === undefined) delete process.env.DSH_DESKTOP_WEB_PROFILE;
    else process.env.DSH_DESKTOP_WEB_PROFILE = previousProfile;
  }
});

test("只把明确的 pnpm modules 或 hoist 错误识别为一次性恢复条件", () => {
  assert.equal(isPnpmCompatibilityFailure("ERR_PNPM_PUBLIC_HOIST_PATTERN_DIFF"), true);
  assert.equal(isPnpmCompatibilityFailure("ERR_PNPM_LOCKFILE_BREAKING_CHANGE"), true);
  assert.equal(isPnpmCompatibilityFailure("ERR_PNPM_VIRTUAL_STORE_DIR_MAX_LENGTH_DIFF"), true);
  assert.equal(isPnpmCompatibilityFailure("ERR_PNPM_UNEXPECTED_STORE"), true);
  assert.equal(isPnpmCompatibilityFailure("ERR_PNPM_FETCH_404"), false);
  assert.equal(isPnpmCompatibilityFailure("ordinary lifecycle failure"), false);
});

test("失败恢复保持全部 profile 控制文件字节一致", () => {
  const profile = mkdtempSync(join(tmpdir(), "dsh-runtime-controls-"));
  const dshHome = mkdtempSync(join(tmpdir(), "dsh-runtime-home-"));
  try {
    const originals = new Map([
      [join(profile, "package.json"), Buffer.from('{"dependencies":{}}\r\n')],
      [join(profile, "pnpm-lock.yaml"), Buffer.from("lockfileVersion: '9.0'\n")],
      [join(profile, "pnpm-workspace.yaml"), Buffer.from("packages:\r\n  - .\r\n")],
      [join(dshHome, "cordis.patch.yml"), Buffer.from("- id: original\n")],
    ]);
    for (const [path, bytes] of originals) writeFileSync(path, bytes);
    const snapshots = captureControlFiles(profile, dshHome);

    for (const path of originals.keys()) writeFileSync(path, "changed\n");
    const optionalPatch = join(profile, "cordis.patch.yml");
    writeFileSync(optionalPatch, "created during failed operation\n");
    restoreControlFiles(snapshots);

    for (const [path, bytes] of originals) assert.deepEqual(readFileSync(path), bytes);
    assert.equal(existsSync(optionalPatch), false);
  } finally {
    rmSync(profile, { recursive: true, force: true });
    rmSync(dshHome, { recursive: true, force: true });
  }
});

test("pnpm major 不兼容只重建一次，重试失败后恢复旧依赖树和全部控制文件", async () => {
  const root = mkdtempSync(join(tmpdir(), "dsh-runtime-recovery-"));
  const profile = join(root, "profile");
  const dshHome = join(root, "home");
  const hostRoot = join(root, "host");
  const fakeCli = join(root, "fake-cli.mjs");
  const calls = join(root, "calls.json");
  mkdirSync(join(profile, "node_modules"), { recursive: true });
  mkdirSync(join(hostRoot, "toolchains", "pnpm-10"), { recursive: true });
  mkdirSync(dshHome, { recursive: true });
  const originals = new Map([
    [join(profile, "package.json"), Buffer.from('{"dependencies":{"existing":"1.0.0"},"dsh":{"profile":{"bundles":["existing"]}}}\r\n')],
    [join(profile, "pnpm-lock.yaml"), Buffer.from("lockfileVersion: '9.0'\n")],
    [join(profile, "pnpm-workspace.yaml"), Buffer.from("packages:\r\n  - .\r\n")],
    [join(profile, "cordis.patch.yml"), Buffer.from("- id: profile-original\n")],
    [join(dshHome, "cordis.patch.yml"), Buffer.from("- id: home-original\n")],
  ]);
  for (const [path, bytes] of originals) writeFileSync(path, bytes);
  writeFileSync(join(profile, "node_modules", "old-tree.txt"), "old dependency tree\n");
  writeFileSync(fakeCli, `
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
const profile = process.env.DSH_DESKTOP_WEB_PROFILE;
const callsPath = process.env.DSH_RUNTIME_TEST_CALLS;
const calls = existsSync(callsPath) ? JSON.parse(readFileSync(callsPath, "utf8")) : [];
const command = process.argv.slice(2).find((value) => value === "add" || value === "install");
calls.push(command);
writeFileSync(callsPath, JSON.stringify(calls));
if (command === "add" && calls.filter((value) => value === "add").length === 1) {
  writeFileSync(join(profile, "package.json"), "first attempt changed package\\n");
  console.error("ERR_PNPM_PUBLIC_HOIST_PATTERN_DIFF");
  process.exit(1);
}
if (command === "install") {
  mkdirSync(join(profile, "node_modules"), { recursive: true });
  writeFileSync(join(profile, "node_modules", "rebuilt-tree.txt"), "rebuilt\\n");
  writeFileSync(join(profile, "pnpm-lock.yaml"), "rebuilt lock\\n");
  process.exit(0);
}
writeFileSync(join(profile, "package.json"), "retry changed package\\n");
writeFileSync(join(profile, "pnpm-workspace.yaml"), "retry changed workspace\\n");
writeFileSync(join(profile, "cordis.patch.yml"), "retry changed profile patch\\n");
writeFileSync(join(process.env.DSH_HOME, "cordis.patch.yml"), "retry changed home patch\\n");
process.exit(2);
`);

  const environment = new Map([
    ["DSH_DESKTOP_NODE_EXECUTABLE", process.execPath],
    ["DSH_DESKTOP_CLI_ENTRY", fakeCli],
    ["DSH_DESKTOP_HOST_ROOT", hostRoot],
    ["DSH_DESKTOP_WEB_PROFILE", profile],
    ["DSH_HOME", dshHome],
    ["DSH_RUNTIME_TEST_CALLS", calls],
  ]);
  const previous = new Map([...environment.keys()].map((name) => [name, process.env[name]]));
  try {
    for (const [name, value] of environment) process.env[name] = value;
    const handle = runPlugin(["add", "fixture"], profile);
    handle.stdout.resume();
    handle.stderr.resume();
    const result = await handle.done;

    assert.equal(result.exitCode, 2);
    assert.deepEqual(JSON.parse(readFileSync(calls, "utf8")), ["add", "install", "add"]);
    for (const [path, bytes] of originals) assert.deepEqual(readFileSync(path), bytes);
    assert.equal(readFileSync(join(profile, "node_modules", "old-tree.txt"), "utf8"), "old dependency tree\n");
    assert.equal(existsSync(join(profile, "node_modules", "rebuilt-tree.txt")), false);
    assert.equal(
      existsSync(join(profile, `.node_modules.dsh-desktop-backup.${process.pid}`)),
      false,
    );
  } finally {
    for (const [name, value] of previous) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
    rmSync(root, { recursive: true, force: true });
  }
});

test("公共 GitHub SSH 地址通过子进程环境回退到 HTTPS", () => {
  const environment = packageManagerEnvironment({
    PATH: "C:\\Windows\\System32",
    npm_config_manage_package_manager_versions: "true",
    GIT_CONFIG_COUNT: "1",
    GIT_CONFIG_KEY_0: "credential.helper",
    GIT_CONFIG_VALUE_0: "manager",
  }, "C:\\runtime\\pnpm-10");

  assert.equal(environment.PATH, `C:\\runtime\\pnpm-10${delimiter}C:\\Windows\\System32`);
  assert.equal(environment.npm_config_manage_package_manager_versions, "false");
  assert.equal(environment.GIT_CONFIG_COUNT, "4");
  assert.equal(environment.GIT_CONFIG_KEY_0, "credential.helper");
  assert.equal(environment.GIT_CONFIG_VALUE_0, "manager");
  assert.deepEqual(
    [1, 2, 3].map((index) => [environment[`GIT_CONFIG_KEY_${index}`], environment[`GIT_CONFIG_VALUE_${index}`]]),
    [
      ["url.https://github.com/.insteadOf", "git+ssh://git@github.com/"],
      ["url.https://github.com/.insteadOf", "ssh://git@github.com/"],
      ["url.https://github.com/.insteadOf", "git@github.com:"],
    ],
  );
});

test("拒绝损坏的 Git 环境配置计数", () => {
  assert.throws(
    () => packageManagerEnvironment({ GIT_CONFIG_COUNT: "invalid" }, "C:\\runtime\\pnpm-10"),
    /GIT_CONFIG_COUNT/,
  );
});

test("成功安装只启用本次新增 bundle", () => {
  const profile = mkdtempSync(join(tmpdir(), "dsh-runtime-profile-"));
  try {
    const manifestPath = join(profile, "package.json");
    writeFileSync(manifestPath, JSON.stringify({
      dependencies: { managed: "1.0.0", inactive: "1.0.0" },
      dsh: { profile: { bundles: ["@deepseek-ai/dsh-base", "managed"] } },
    }));
    const before = readProfileSnapshot(profile);
    writeFileSync(manifestPath, JSON.stringify({
      dependencies: { managed: "1.0.0", inactive: "1.0.0", "dsh-pet": "^0.1.4" },
      dsh: { profile: { bundles: ["managed", "inactive", "dsh-pet"] } },
    }));

    assert.equal(reconcileProfileBundles(profile, before, true), true);
    const after = JSON.parse(readFileSync(manifestPath, "utf8"));
    assert.deepEqual(after.dsh.profile.bundles, ["@deepseek-ai/dsh-base", "managed", "dsh-pet"]);
    assert.equal(after.dependencies.inactive, "1.0.0");
  } finally {
    rmSync(profile, { recursive: true, force: true });
  }
});

test("失败操作恢复原 bundle 状态", () => {
  const profile = mkdtempSync(join(tmpdir(), "dsh-runtime-profile-"));
  try {
    const manifestPath = join(profile, "package.json");
    writeFileSync(manifestPath, JSON.stringify({
      dependencies: { managed: "1.0.0", inactive: "1.0.0" },
      dsh: { profile: { bundles: ["managed"] } },
    }));
    const before = readProfileSnapshot(profile);
    writeFileSync(manifestPath, JSON.stringify({
      dependencies: { managed: "1.0.0", inactive: "1.0.0" },
      dsh: { profile: { bundles: ["managed", "inactive"] } },
    }));

    reconcileProfileBundles(profile, before, false);
    const after = JSON.parse(readFileSync(manifestPath, "utf8"));
    assert.deepEqual(after.dsh.profile.bundles, ["managed"]);
  } finally {
    rmSync(profile, { recursive: true, force: true });
  }
});
