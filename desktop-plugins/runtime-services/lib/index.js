import { readFileSync } from "node:fs";
import { spawn } from "node:child_process";
import { delimiter, isAbsolute, join } from "node:path";

const PROFILE_NAME = "web";
const SUPPORTED_PNPM_MAJORS = new Set([9, 10, 11]);
let activeOperation = null;

function requiredAbsolutePath(name) {
  const value = process.env[name];
  if (typeof value !== "string" || !isAbsolute(value) || value.includes("\0")) {
    throw new Error(`${name} must be an absolute path`);
  }
  return value;
}

/** 校验识别到的 pnpm major，避免用未知运行时改写现有 profile。 */
function assertSupportedPnpmMajor(major) {
  if (!SUPPORTED_PNPM_MAJORS.has(major)) {
    throw new Error(`unsupported profile pnpm major: ${major}; supported majors are 9, 10 and 11`);
  }
  return major;
}

/** 从 packageManager 字符串读取 pnpm major。 */
function packageManagerMajor(value) {
  if (typeof value !== "string") return null;
  const match = /^pnpm@(\d+)(?:\.|\+|$)/.exec(value.trim());
  return match === null ? null : assertSupportedPnpmMajor(Number(match[1]));
}

/** 从 pnpm modules 元数据读取创建该 profile 的 pnpm major。 */
function selectPnpmMajor(text) {
  if (typeof text !== "string" || text.trim() === "") return 11;

  // pnpm 当前把 .modules.yaml 写成 JSON；先结构化解析，避免依赖展示格式。
  try {
    const metadata = JSON.parse(text);
    const managerMajor = packageManagerMajor(metadata?.packageManager);
    if (managerMajor !== null) return managerMajor;
    const storeMatch = /[\\/]store[\\/]v(\d+)(?:[\\/]|$)/i.exec(metadata?.storeDir ?? "");
    if (storeMatch !== null) return assertSupportedPnpmMajor(Number(storeMatch[1]));
  } catch (error) {
    if (!(error instanceof SyntaxError)) throw error;
  }

  // 兼容旧 pnpm 写出的 YAML，以及人为维护的最小 modules 元数据。
  const managerMatch = /^\s*["']?packageManager["']?\s*:\s*["']?(pnpm@\d+(?:\.[^\s"']*)?)/m.exec(text);
  const managerMajor = packageManagerMajor(managerMatch?.[1]);
  if (managerMajor !== null) return managerMajor;

  const storeMatch = /^\s*["']?storeDir["']?\s*:\s*["']?.*?[\\/]store[\\/]v(\d+)(?:[\\/"']|$)/im.exec(text);
  if (storeMatch !== null) return assertSupportedPnpmMajor(Number(storeMatch[1]));
  return 11;
}

/** 缺少 modules 元数据的新 profile 使用当前固定的 pnpm 11。 */
function profilePnpmMajor(profileDirectory) {
  try {
    return selectPnpmMajor(readFileSync(join(profileDirectory, "node_modules", ".modules.yaml"), "utf8"));
  } catch (error) {
    if (error?.code === "ENOENT") return 11;
    throw error;
  }
}

function terminateProcessTree(child) {
  if (child.exitCode !== null || child.pid === undefined) return;
  if (process.platform === "win32") {
    const killer = spawn("taskkill", ["/PID", String(child.pid), "/T", "/F"], {
      stdio: "ignore",
      windowsHide: true,
    });
    killer.once("error", () => child.kill("SIGKILL"));
    return;
  }
  child.kill("SIGTERM");
}

/** 为 Market 启动一次受控的 dsh plugin 子进程，并锁住并发安装操作。 */
function runPlugin(args, invokingDirectory, signal) {
  if (activeOperation !== null) {
    throw new Error("another desktop pnpm operation is already running");
  }
  if (!Array.isArray(args) || !args.every((value) => typeof value === "string" && !value.includes("\0"))) {
    throw new Error("invalid desktop pnpm arguments");
  }
  if (!isAbsolute(invokingDirectory) || invokingDirectory.includes("\0")) {
    throw new Error("desktop pnpm invoking directory must be absolute");
  }

  const node = requiredAbsolutePath("DSH_DESKTOP_NODE_EXECUTABLE");
  const cli = requiredAbsolutePath("DSH_DESKTOP_CLI_ENTRY");
  const hostRoot = requiredAbsolutePath("DSH_DESKTOP_HOST_ROOT");
  const profileDirectory = requiredAbsolutePath("DSH_DESKTOP_WEB_PROFILE");
  const major = profilePnpmMajor(profileDirectory);
  const toolchain = join(hostRoot, "toolchains", `pnpm-${major}`);
  const child = spawn(node, [cli, "plugin", "--profile", PROFILE_NAME, ...args], {
    cwd: invokingDirectory,
    env: {
      ...process.env,
      PATH: `${toolchain}${delimiter}${process.env.PATH ?? ""}`,
    },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });

  let cancelled = false;
  const cancel = () => {
    if (cancelled) return;
    cancelled = true;
    terminateProcessTree(child);
  };
  signal?.addEventListener("abort", cancel, { once: true });
  const done = new Promise((resolve) => {
    child.once("error", () => resolve({ exitCode: 127, signal: null }));
    child.once("close", (exitCode, closeSignal) => resolve({ exitCode, signal: closeSignal }));
  }).finally(() => {
    signal?.removeEventListener("abort", cancel);
    if (activeOperation?.child === child) activeOperation = null;
  });

  const handle = { stdout: child.stdout, stderr: child.stderr, done, cancel, child };
  activeOperation = handle;
  return handle;
}

/** 在所有 Loader entry 之前发布 Market 依赖的桌面 profile 与包管理服务。 */
function apply(ctx) {
  const profileDirectory = requiredAbsolutePath("DSH_DESKTOP_WEB_PROFILE");
  ctx.provide("desktopProfiles", Object.freeze({
    current: Object.freeze({ name: PROFILE_NAME, dir: profileDirectory }),
  }));
  ctx.provide("desktopPnpm", Object.freeze({ runPlugin }));
}

const inject = [];

export { apply, inject, profilePnpmMajor, runPlugin, selectPnpmMajor };
