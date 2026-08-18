import {
  existsSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { spawn } from "node:child_process";
import { delimiter, isAbsolute, join } from "node:path";
import { PassThrough } from "node:stream";

const PROFILE_NAME = "web";
const FIXED_PNPM_DIRECTORY = "pnpm-10";
const PROFILE_CONTROL_FILES = [
  "package.json",
  "pnpm-lock.yaml",
  "pnpm-workspace.yaml",
  "cordis.patch.yml",
];
const PNPM_COMPATIBILITY_ERRORS = [
  "ERR_PNPM_PUBLIC_HOIST_PATTERN_DIFF",
  "ERR_PNPM_LOCKFILE_BREAKING_CHANGE",
  "ERR_PNPM_VIRTUAL_STORE_DIR_MAX_LENGTH_DIFF",
  "ERR_PNPM_UNEXPECTED_STORE",
  "modules directory was created using a different hoist-pattern",
  "modules directory is not compatible with the current pnpm",
];
const GITHUB_HTTPS_REWRITES = [
  "git+ssh://git@github.com/",
  "ssh://git@github.com/",
  "git@github.com:",
];
let activeOperation = null;
let coreReadyUrl = null;

function requiredAbsolutePath(name) {
  const value = process.env[name];
  if (typeof value !== "string" || !isAbsolute(value) || value.includes("\0")) {
    throw new Error(`${name} must be an absolute path`);
  }
  return value;
}

/**
 * 为 Market 的公共 GitHub 依赖增加 HTTPS 兜底，同时保留调用方已有的 Git 配置。
 * pnpm 会把 `github:` 简写重新解析成 SSH URL；未配置 GitHub SSH 密钥时，任意旧依赖都会阻断整个 profile 的安装。
 */
function packageManagerEnvironment(baseEnvironment, toolchain) {
  const environment = {
    ...baseEnvironment,
    PATH: `${toolchain}${delimiter}${baseEnvironment.PATH ?? ""}`,
    // pnpm 10 默认会服从 profile 的 packageManager 字段下载旧 major；桌面必须始终执行内置版本。
    npm_config_manage_package_manager_versions: "false",
    COREPACK_ENABLE_PROJECT_SPEC: "0",
    COREPACK_ENABLE_DOWNLOAD_PROMPT: "0",
  };
  const rawCount = baseEnvironment.GIT_CONFIG_COUNT;
  const existingCount = rawCount === undefined ? 0 : Number(rawCount);
  if (!Number.isSafeInteger(existingCount) || existingCount < 0 || existingCount > 1024) {
    throw new Error("GIT_CONFIG_COUNT must be an integer between 0 and 1024");
  }
  GITHUB_HTTPS_REWRITES.forEach((source, offset) => {
    const index = existingCount + offset;
    environment[`GIT_CONFIG_KEY_${index}`] = "url.https://github.com/.insteadOf";
    environment[`GIT_CONFIG_VALUE_${index}`] = source;
  });
  environment.GIT_CONFIG_COUNT = String(existingCount + GITHUB_HTTPS_REWRITES.length);
  return environment;
}

/** 读取包操作前的直接依赖与 bundle 状态，避免 DSH CLI 顺带启用历史依赖。 */
function readProfileSnapshot(profileDirectory) {
  const manifest = JSON.parse(readFileSync(join(profileDirectory, "package.json"), "utf8"));
  return {
    dependencies: Object.keys(manifest.dependencies ?? {}),
    bundles: Array.isArray(manifest.dsh?.profile?.bundles)
      ? manifest.dsh.profile.bundles.filter((value) => typeof value === "string")
      : [],
  };
}

/** 捕获 profile 与全局 Cordis 控制文件的原始字节，不解析也不重排内容。 */
function captureControlFiles(profileDirectory, dshHome) {
  const paths = PROFILE_CONTROL_FILES.map((name) => join(profileDirectory, name));
  paths.push(join(dshHome, "cordis.patch.yml"));
  return paths.map((path) => ({
    path,
    bytes: existsSync(path) ? readFileSync(path) : null,
  }));
}

/** 使用同目录临时文件恢复快照；原先不存在的文件会被删除。 */
function restoreControlFiles(snapshots) {
  for (const snapshot of snapshots) {
    if (snapshot.bytes === null) {
      rmSync(snapshot.path, { force: true });
      continue;
    }
    const temporary = `${snapshot.path}.${process.pid}.${Date.now()}.restore`;
    writeFileSync(temporary, snapshot.bytes);
    renameSync(temporary, snapshot.path);
  }
}

/** 只识别 pnpm 明确报告的 modules/hoist major 不兼容，不把普通安装失败误判为迁移。 */
function isPnpmCompatibilityFailure(output) {
  return PNPM_COMPATIBILITY_ERRORS.some((marker) => output.includes(marker));
}

/**
 * 归一化 DSH CLI 改写后的 bundle：成功时只追加本次新增依赖，失败时恢复操作前状态。
 * 依赖字段本身完全由官方 CLI 与 Market 事务管理，本函数只收敛 bundle membership。
 */
function reconcileProfileBundles(profileDirectory, before, succeeded) {
  const manifestPath = join(profileDirectory, "package.json");
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  const afterDependencies = Object.keys(manifest.dependencies ?? {});
  const afterDependencySet = new Set(afterDependencies);
  const beforeDependencySet = new Set(before.dependencies);
  const removedDependencies = new Set(before.dependencies.filter((name) => !afterDependencySet.has(name)));
  const desired = succeeded
    ? before.bundles.filter((name) => !removedDependencies.has(name))
    : [...before.bundles];

  if (succeeded) {
    for (const name of afterDependencies) {
      if (!beforeDependencySet.has(name) && !desired.includes(name)) desired.push(name);
    }
  }

  const current = Array.isArray(manifest.dsh?.profile?.bundles) ? manifest.dsh.profile.bundles : [];
  if (current.length === desired.length && current.every((name, index) => name === desired[index])) return false;

  manifest.dsh ??= {};
  manifest.dsh.profile ??= {};
  manifest.dsh.profile.bundles = desired;
  const temporaryPath = `${manifestPath}.${process.pid}.${Date.now()}.tmp`;
  writeFileSync(temporaryPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  renameSync(temporaryPath, manifestPath);
  return true;
}

function terminateProcessTree(child) {
  if (child === null || child.exitCode !== null || child.pid === undefined) return;
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

/** 启动一次 DSH plugin 子进程，将输出转发给 Market，同时保留兼容错误判定所需的有限文本。 */
function runPluginChild(args, invokingDirectory, environment, output, operation) {
  return new Promise((resolve) => {
    const node = requiredAbsolutePath("DSH_DESKTOP_NODE_EXECUTABLE");
    const cli = requiredAbsolutePath("DSH_DESKTOP_CLI_ENTRY");
    const child = spawn(node, [cli, "plugin", "--profile", PROFILE_NAME, ...args], {
      cwd: invokingDirectory,
      env: environment,
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    operation.child = child;
    const stdout = [];
    const stderr = [];
    child.stdout.on("data", (chunk) => {
      stdout.push(Buffer.from(chunk));
      output.stdout.write(chunk);
    });
    child.stderr.on("data", (chunk) => {
      stderr.push(Buffer.from(chunk));
      output.stderr.write(chunk);
    });
    let settled = false;
    const finish = (exitCode, closeSignal) => {
      if (settled) return;
      settled = true;
      resolve({
        exitCode,
        signal: closeSignal,
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: Buffer.concat(stderr).toString("utf8"),
      });
    };
    child.once("error", (error) => {
      output.stderr.write(`desktop runtime failed to start plugin command: ${error.message}\n`);
      finish(127, null);
    });
    child.once("close", finish);
  });
}

/**
 * 执行固定 pnpm 10 操作；仅在 major 不兼容时重建一次，并对失败路径恢复控制文件与旧依赖树。
 */
async function runPluginTransaction(args, invokingDirectory, output, operation) {
  const hostRoot = requiredAbsolutePath("DSH_DESKTOP_HOST_ROOT");
  const profileDirectory = requiredAbsolutePath("DSH_DESKTOP_WEB_PROFILE");
  const dshHome = requiredAbsolutePath("DSH_HOME");
  const toolchain = join(hostRoot, "toolchains", FIXED_PNPM_DIRECTORY);
  const environment = packageManagerEnvironment(process.env, toolchain);
  const profileBefore = readProfileSnapshot(profileDirectory);
  const controls = captureControlFiles(profileDirectory, dshHome);
  const modules = join(profileDirectory, "node_modules");
  const backup = join(profileDirectory, `.node_modules.dsh-desktop-backup.${process.pid}.${Date.now()}`);

  let result = await runPluginChild(args, invokingDirectory, environment, output, operation);
  const firstOutput = `${result.stderr}\n${result.stdout}`;
  let dependencyTreeMoved = false;
  if (!operation.cancelled && result.exitCode !== 0 && isPnpmCompatibilityFailure(firstOutput)) {
    output.stderr.write("desktop runtime detected incompatible pnpm metadata; rebuilding once with pnpm 10\n");
    restoreControlFiles(controls);
    if (existsSync(modules)) {
      renameSync(modules, backup);
      dependencyTreeMoved = true;
    }
    const rebuild = await runPluginChild(
      ["install", "--no-frozen-lockfile"],
      invokingDirectory,
      environment,
      output,
      operation,
    );
    if (!operation.cancelled && rebuild.exitCode === 0 && rebuild.signal === null) {
      result = await runPluginChild(args, invokingDirectory, environment, output, operation);
    } else {
      result = rebuild;
    }
  }

  const succeeded = result.exitCode === 0 && result.signal === null && !operation.cancelled;
  if (succeeded) {
    try {
      reconcileProfileBundles(profileDirectory, profileBefore, true);
    } catch (error) {
      restoreControlFiles(controls);
      if (dependencyTreeMoved) {
        rmSync(modules, { recursive: true, force: true });
        renameSync(backup, modules);
      }
      throw error;
    }
    if (dependencyTreeMoved) {
      try {
        rmSync(backup, { recursive: true, force: true });
      } catch (error) {
        output.stderr.write(`desktop runtime could not remove recovered dependency backup: ${error.message}\n`);
      }
    }
  } else {
    restoreControlFiles(controls);
    if (dependencyTreeMoved) {
      rmSync(modules, { recursive: true, force: true });
      renameSync(backup, modules);
    }
    output.stderr.write(
      "desktop runtime restored profile control files and dependency tree; third-party script side effects outside the profile are not transactional\n",
    );
  }
  return { exitCode: succeeded ? 0 : result.exitCode, signal: result.signal };
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

  const stdout = new PassThrough();
  const stderr = new PassThrough();
  const operation = { child: null, cancelled: false };
  const cancel = () => {
    if (operation.cancelled) return;
    operation.cancelled = true;
    terminateProcessTree(operation.child);
  };
  signal?.addEventListener("abort", cancel, { once: true });
  const handle = { stdout, stderr, done: null, cancel, child: null };
  activeOperation = handle;
  handle.done = runPluginTransaction(args, invokingDirectory, { stdout, stderr }, operation)
    .catch((error) => {
      stderr.write(`desktop runtime plugin transaction failed: ${error.stack ?? error}\n`);
      return { exitCode: 1, signal: null };
    })
    .finally(() => {
      stdout.end();
      stderr.end();
    signal?.removeEventListener("abort", cancel);
      if (activeOperation === handle) activeOperation = null;
  });
  Object.defineProperty(handle, "child", { get: () => operation.child });
  return handle;
}

/** 在所有 Loader entry 之前发布 Market 依赖的桌面 profile 与包管理服务。 */
function apply(ctx) {
  const profileDirectory = requiredAbsolutePath("DSH_DESKTOP_WEB_PROFILE");
  ctx.provide("desktopProfiles", Object.freeze({
    current: Object.freeze({ name: PROFILE_NAME, dir: profileDirectory }),
  }));
  ctx.provide("desktopPnpm", Object.freeze({ runPlugin }));
  const host = ctx.webServer?.host;
  const port = ctx.webServer?.port;
  if (ctx.webRuntime && (host === "127.0.0.1" || host === "localhost") && Number.isInteger(port)) {
    const url = `http://${host}:${port}`;
    if (coreReadyUrl === null) {
      coreReadyUrl = url;
      console.log(`dsh desktop-core: ${url}`);
    } else if (coreReadyUrl !== url) {
      throw new Error(`desktop core reported conflicting URLs: ${coreReadyUrl} and ${url}`);
    }
  }
}

const inject = ["webServer", "webRuntime"];

export {
  apply,
  captureControlFiles,
  isPnpmCompatibilityFailure,
  inject,
  packageManagerEnvironment,
  readProfileSnapshot,
  reconcileProfileBundles,
  restoreControlFiles,
  runPlugin,
};
