import { readFile, rename, unlink, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join } from "node:path";

const API_PREFIX = "/api/desktop-managed-plugins";
const CONTROL_BUNDLE = "@dsh-desktop/theme-settings";
const PROTECTED_BUNDLES = new Set([
  "@deepseek-ai/dsh-base",
  "@deepseek-ai/dsh-web-app",
  "dshmarket",
  "dshmarket-desktop",
  CONTROL_BUNDLE,
]);
const MANAGED_PLUGINS = Object.freeze([
  { package: "dsh-at-file", label: "@ 文件引用" },
  { package: "@omdsh-dev/dsh-genui", label: "GenUI" },
  { package: "dsh-better-sidebar", label: "Better Sidebar" },
  { package: "@linxin666/dsh-skins", label: "主题皮肤" },
  { package: "@vectorize-io/hindsight-coding-agents", label: "Hindsight 记忆" },
  { package: "@liustack/modlens", label: "ModLens 视觉" },
  { package: "@zebbkira/dsh-skills-mcp-manager", label: "Skills / MCP Manager" },
]);
const TOGGLEABLE = new Set(MANAGED_PLUGINS.map((item) => item.package));
let writeTail = Promise.resolve();
let temporarySequence = 0;

function json(res, status, body) {
  res.writeHead(status, { "content-type": "application/json; charset=utf-8" });
  res.end(JSON.stringify(body));
}

function requireMethod(req, res, method) {
  if (req.method === method) return true;
  json(res, 405, { ok: false, error: "method-not-allowed" });
  return false;
}

/** 拒绝浏览器跨站请求，避免任意网页借 localhost 修改用户 profile。 */
function isSameOriginRequest(req) {
  const host = req.headers.host;
  if (typeof host !== "string" || host === "") return false;
  let hostname;
  try {
    hostname = new URL(`http://${host}`).hostname;
  } catch {
    return false;
  }
  if (!["127.0.0.1", "localhost", "[::1]"].includes(hostname)) return false;
  if (req.headers["sec-fetch-site"] === "cross-site") return false;
  const origin = req.headers.origin;
  if (typeof origin !== "string" || origin === "" || origin === "null") return true;
  try {
    return new URL(origin).host === host;
  } catch {
    return false;
  }
}

function requireSameOrigin(req, res) {
  if (isSameOriginRequest(req)) return true;
  json(res, 403, { ok: false, error: "cross-site-request-rejected" });
  return false;
}

/** 有界读取 JSON，避免本地 API 被超大请求拖垮。 */
function readJsonBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    req.on("data", (chunk) => {
      size += chunk.length;
      if (size > 64 * 1024) {
        reject(new Error("body-too-large"));
        queueMicrotask(() => req.destroy());
        return;
      }
      chunks.push(chunk);
    });
    req.on("end", () => {
      try {
        resolve(chunks.length === 0 ? {} : JSON.parse(Buffer.concat(chunks).toString("utf8")));
      } catch {
        reject(new Error("invalid-json"));
      }
    });
    req.on("error", reject);
  });
}

function profilePath() {
  return join(process.env.DSH_HOME || join(homedir(), ".dsh"), "profiles", "web", "package.json");
}

function profileBundles(profile) {
  const bundles = profile?.dsh?.profile?.bundles;
  if (!Array.isArray(bundles) || !bundles.every((item) => typeof item === "string")) {
    throw new Error("invalid-web-profile");
  }
  return bundles;
}

/** 按桌面托管顺序插回一个 bundle，同时保留所有未知用户 bundle 的相对位置。 */
function enableManagedBundle(bundles, packageName) {
  const next = bundles.filter((bundle) => bundle !== packageName);
  const targetOrder = MANAGED_PLUGINS.findIndex((item) => item.package === packageName);
  const later = new Set(MANAGED_PLUGINS.slice(targetOrder + 1).map((item) => item.package));
  const before = next.findIndex((bundle) => later.has(bundle));
  if (before >= 0) {
    next.splice(before, 0, packageName);
    return next;
  }
  const known = new Set([
    "dshmarket",
    CONTROL_BUNDLE,
    ...MANAGED_PLUGINS.slice(0, targetOrder).map((item) => item.package),
  ]);
  let after = -1;
  for (let index = 0; index < next.length; index += 1) {
    if (known.has(next[index])) after = index;
  }
  next.splice(after + 1, 0, packageName);
  return next;
}

function assertToggleBody(body) {
  const keys = Object.keys(body).sort().join(",");
  if (keys !== "enabled,package,profile" || body.profile !== "web") {
    throw new Error("invalid-request");
  }
  if (typeof body.package !== "string" || typeof body.enabled !== "boolean") {
    throw new Error("invalid-request");
  }
  if (PROTECTED_BUNDLES.has(body.package)) throw new Error("protected-bundle");
  if (!TOGGLEABLE.has(body.package)) throw new Error("unknown-managed-plugin");
}

async function readProfile(path = profilePath()) {
  const profile = JSON.parse(await readFile(path, "utf8"));
  profileBundles(profile);
  return profile;
}

/** 使用同目录临时文件替换 profile；失败时清理临时文件。 */
async function atomicWriteProfile(path, profile) {
  temporarySequence += 1;
  const temporary = join(dirname(path), `.package.json.${process.pid}.${temporarySequence}.tmp`);
  try {
    await writeFile(temporary, `${JSON.stringify(profile, null, 2)}\n`, "utf8");
    await rename(temporary, path);
  } catch (error) {
    await unlink(temporary).catch(() => {});
    throw error;
  }
}

function serializeWrite(operation) {
  const current = writeTail.then(operation, operation);
  writeTail = current.catch(() => {});
  return current;
}

async function listManagedPlugins(path = profilePath()) {
  const profile = await readProfile(path);
  const enabled = new Set(profileBundles(profile));
  return MANAGED_PLUGINS.map((item) => ({ ...item, enabled: enabled.has(item.package) }));
}

async function toggleManagedPlugin(body, path = profilePath()) {
  assertToggleBody(body);
  return serializeWrite(async () => {
    const profile = await readProfile(path);
    const bundles = profileBundles(profile);
    const next = body.enabled
      ? enableManagedBundle(bundles, body.package)
      : bundles.filter((bundle) => bundle !== body.package);
    if (next.length !== bundles.length || next.some((bundle, index) => bundle !== bundles[index])) {
      profile.dsh.profile.bundles = next;
      await atomicWriteProfile(path, profile);
    }
    return {
      ok: true,
      package: body.package,
      enabled: next.includes(body.package),
      restartRequired: true,
    };
  });
}

function makeRoutes() {
  return [
    {
      kind: "exact",
      path: `${API_PREFIX}/state`,
      handler: (req, res) => {
        if (!requireMethod(req, res, "GET") || !requireSameOrigin(req, res)) return;
        listManagedPlugins().then(
          (plugins) => json(res, 200, { ok: true, profile: "web", plugins }),
          (error) => json(res, 500, { ok: false, error: error.message }),
        );
      },
    },
    {
      kind: "exact",
      path: `${API_PREFIX}/toggle`,
      handler: (req, res) => {
        if (!requireMethod(req, res, "POST") || !requireSameOrigin(req, res)) return Promise.resolve();
        return readJsonBody(req).then(
          (body) => toggleManagedPlugin(body).then(
            (value) => json(res, 200, value),
            (error) => json(res, 400, { ok: false, error: error.message }),
          ),
          (error) => json(res, 400, { ok: false, error: error.message }),
        );
      },
    },
  ];
}

const inject = ["webServer"];

/** 注册桌面托管插件状态与开关 API；路由失败不应拖垮核心 DSH。 */
function apply(ctx) {
  try {
    ctx.effect(() => {
      const disposers = makeRoutes().map((route) => ctx.webServer.register(route));
      return () => disposers.forEach((dispose) => dispose());
    }, "desktop-theme-settings: managed plugin routes");
  } catch (error) {
    console.error("[desktop-theme-settings] route registration failed:", error);
  }
}

export {
  API_PREFIX,
  CONTROL_BUNDLE,
  MANAGED_PLUGINS,
  PROTECTED_BUNDLES,
  apply,
  assertToggleBody,
  enableManagedBundle,
  inject,
  isSameOriginRequest,
  listManagedPlugins,
  makeRoutes,
  toggleManagedPlugin,
};
