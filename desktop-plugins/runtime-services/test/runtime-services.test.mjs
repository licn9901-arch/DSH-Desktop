import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { test } from "node:test";
import { delimiter, join } from "node:path";
import {
  packageManagerEnvironment,
  readProfileSnapshot,
  reconcileProfileBundles,
  selectPnpmMajor,
} from "../lib/index.js";

test("从 modules 元数据选择固定 pnpm major", () => {
  assert.equal(selectPnpmMajor("packageManager: pnpm@9.15.9\n"), 9);
  assert.equal(selectPnpmMajor("packageManager: 'pnpm@10.33.2+sha512-test'\n"), 10);
  assert.equal(selectPnpmMajor("packageManager: pnpm@11.22.0\n"), 11);
  assert.equal(selectPnpmMajor(JSON.stringify({
    layoutVersion: 5,
    packageManager: "pnpm@10.33.2",
    storeDir: "C:\\Users\\tester\\AppData\\Local\\pnpm\\store\\v10",
  })), 10);
  assert.equal(selectPnpmMajor('storeDir: C:\\pnpm\\store\\v9\n'), 9);
  assert.equal(selectPnpmMajor(""), 11);
});

test("拒绝没有固定运行时的 pnpm major", () => {
  assert.throws(() => selectPnpmMajor("packageManager: pnpm@12.0.0\n"), /unsupported profile pnpm major/);
  assert.throws(
    () => selectPnpmMajor(JSON.stringify({ storeDir: "C:\\pnpm\\store\\v12" })),
    /unsupported profile pnpm major/,
  );
});

test("公共 GitHub SSH 地址通过子进程环境回退到 HTTPS", () => {
  const environment = packageManagerEnvironment({
    PATH: "C:\\Windows\\System32",
    GIT_CONFIG_COUNT: "1",
    GIT_CONFIG_KEY_0: "credential.helper",
    GIT_CONFIG_VALUE_0: "manager",
  }, "C:\\runtime\\pnpm-10");

  assert.equal(environment.PATH, `C:\\runtime\\pnpm-10${delimiter}C:\\Windows\\System32`);
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
