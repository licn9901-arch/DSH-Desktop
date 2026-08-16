import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, test } from "node:test";
import {
  assertToggleBody,
  isSameOriginRequest,
  listManagedPlugins,
  toggleManagedPlugin,
} from "../lib/index.js";

const directories = [];

async function fixture() {
  const directory = await mkdtemp(join(tmpdir(), "dsh-desktop-plugin-controls-"));
  directories.push(directory);
  const path = join(directory, "package.json");
  const profile = {
    dependencies: {
      "dsh-at-file": "link:C:/managed/dsh-at-file",
      "user-plugin": "1.2.3",
    },
    dsh: {
      profile: {
        bundles: [
          "@deepseek-ai/dsh-base",
          "@deepseek-ai/dsh-web-app",
          "dshmarket",
          "@dsh-desktop/theme-settings",
          "dsh-at-file",
          "@omdsh-dev/dsh-genui",
          "user-plugin",
        ],
      },
    },
  };
  await writeFile(path, JSON.stringify(profile), "utf8");
  return { path, profile };
}

afterEach(async () => {
  await Promise.all(directories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

test("只接受 web profile 白名单，并保护基础 bundle", () => {
  assert.throws(
    () => assertToggleBody({ profile: "web", package: "dshmarket", enabled: false }),
    /protected-bundle/,
  );
  assert.throws(
    () => assertToggleBody({ profile: "web", package: "unknown-plugin", enabled: false }),
    /unknown-managed-plugin/,
  );
  assert.throws(
    () => assertToggleBody({ profile: "other", package: "dsh-at-file", enabled: false }),
    /invalid-request/,
  );
});

test("API 只接受回环 Host 与同源浏览器请求", () => {
  assert.equal(
    isSameOriginRequest({
      headers: {
        host: "127.0.0.1:3210",
        origin: "http://127.0.0.1:3210",
        "sec-fetch-site": "same-origin",
      },
    }),
    true,
  );
  assert.equal(
    isSameOriginRequest({ headers: { host: "192.168.1.10:3210" } }),
    false,
  );
  assert.equal(
    isSameOriginRequest({
      headers: {
        host: "localhost:3210",
        origin: "https://attacker.example",
        "sec-fetch-site": "cross-site",
      },
    }),
    false,
  );
});

test("开关只修改 bundles，保留 dependencies 与未知用户 bundle", async () => {
  const { path, profile } = await fixture();
  await toggleManagedPlugin(
    { profile: "web", package: "dsh-at-file", enabled: false },
    path,
  );
  const updated = JSON.parse(await readFile(path, "utf8"));
  assert.deepEqual(updated.dependencies, profile.dependencies);
  assert(updated.dsh.profile.bundles.includes("user-plugin"));
  assert(!updated.dsh.profile.bundles.includes("dsh-at-file"));

  await toggleManagedPlugin(
    { profile: "web", package: "dsh-at-file", enabled: false },
    path,
  );
  const repeated = JSON.parse(await readFile(path, "utf8"));
  assert.deepEqual(repeated, updated);
});

test("并发开关串行合并，列表返回最终状态", async () => {
  const { path } = await fixture();
  await Promise.all([
    toggleManagedPlugin(
      { profile: "web", package: "dsh-at-file", enabled: false },
      path,
    ),
    toggleManagedPlugin(
      { profile: "web", package: "@omdsh-dev/dsh-genui", enabled: false },
      path,
    ),
  ]);
  const rows = await listManagedPlugins(path);
  assert.equal(rows.find((row) => row.package === "dsh-at-file").enabled, false);
  assert.equal(rows.find((row) => row.package === "@omdsh-dev/dsh-genui").enabled, false);
});
