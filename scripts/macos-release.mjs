import { createHash } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  cpSync,
  createReadStream,
  createWriteStream,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  readlinkSync,
  renameSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn, spawnSync } from "node:child_process";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(scriptDirectory, "..");
const resourcesRoot = join(projectRoot, "src-tauri", "resources");
const cacheRoot = join(projectRoot, ".runtime-cache", "macos");
const reportRoot = join(projectRoot, ".release-work", "macos", "reports");
const gatedRoot = join(projectRoot, ".release-work", "macos", "gated");
const packageJson = readJson(join(projectRoot, "package.json"));
const runtimeLock = readJson(join(projectRoot, "runtime.lock.json"));
const macosLock = readJson(join(projectRoot, "runtime.macos.lock.json"));
const pluginLock = readJson(join(projectRoot, "plugins.lock.json"));

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function writeJson(path, value) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function assertProjectPath(path) {
  const resolvedPath = resolve(path);
  if (resolvedPath !== projectRoot && !resolvedPath.startsWith(`${projectRoot}${sep}`)) {
    throw new Error(`refusing to modify a path outside the project: ${resolvedPath}`);
  }
  return resolvedPath;
}

function assertMacosHost() {
  if (process.platform !== "darwin" || process.arch !== "arm64") {
    throw new Error(`macOS release requires a darwin-arm64 host, got ${process.platform}-${process.arch}`);
  }
  if (macosLock.schemaVersion !== 1 || macosLock.platform !== "darwin-arm64") {
    throw new Error("runtime.macos.lock.json must declare schema 1 darwin-arm64");
  }
  if (macosLock.node.version !== runtimeLock.node.version) {
    throw new Error("macOS and Windows runtime locks must pin the same Node version");
  }
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd ?? projectRoot,
    env: { ...process.env, ...options.env },
    encoding: "utf8",
    stdio: options.capture ? "pipe" : "inherit",
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (!options.allowFailure && result.status !== 0) {
    const detail = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    throw new Error(`${command} ${args.join(" ")} failed with ${result.status}${detail ? `\n${detail}` : ""}`);
  }
  return result;
}

async function sha256File(path) {
  const digest = createHash("sha256");
  for await (const chunk of createReadStream(path)) digest.update(chunk);
  return digest.digest("hex");
}

async function downloadLocked(url, destination, expectedSha256) {
  mkdirSync(dirname(destination), { recursive: true });
  if (!existsSync(destination)) {
    const temporary = `${destination}.${process.pid}.tmp`;
    rmSync(temporary, { force: true });
    const response = await fetch(url, { headers: { "user-agent": "DSH-Desktop-build" } });
    if (!response.ok || !response.body) {
      throw new Error(`download failed (${response.status}) for ${url}`);
    }
    await pipeline(Readable.fromWeb(response.body), createWriteStream(temporary));
    renameSync(temporary, destination);
  }
  const actual = await sha256File(destination);
  if (actual !== expectedSha256.toLowerCase()) {
    throw new Error(`SHA-256 mismatch for ${destination}: expected ${expectedSha256}, got ${actual}`);
  }
}

function copyDirectoryContents(source, destination) {
  mkdirSync(destination, { recursive: true });
  for (const entry of readdirSync(source)) {
    cpSync(join(source, entry), join(destination, entry), {
      recursive: true,
      dereference: false,
      preserveTimestamps: false,
    });
  }
}

const developmentDirectories = new Set([
  "test", "tests", "__tests__", "example", "examples", "benchmark", "benchmarks",
]);
const developmentExtensions = [".pdb", ".map", ".tsx", ".d.ts", ".d.cts", ".d.mts", ".ts"];

function isDevelopmentFile(name) {
  const lower = name.toLowerCase();
  if (developmentExtensions.some((extension) => lower.endsWith(extension))) return true;
  return lower.endsWith(".md")
    && !/^(license|notice|copying)/i.test(name)
    && name !== "SKILL.md";
}

function removeDevelopmentFiles(root) {
  function visit(directory) {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) {
        if (developmentDirectories.has(entry.name.toLowerCase())) {
          rmSync(path, { recursive: true, force: true });
        } else {
          visit(path);
        }
      } else if (entry.isFile() && isDevelopmentFile(entry.name)) {
        rmSync(path, { force: true });
      }
    }
  }
  visit(root);
}

function reduceNodePty(nodeModules) {
  const root = join(nodeModules, "node-pty");
  if (!existsSync(root)) return;
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory() && !["lib", "prebuilds"].includes(entry.name)) {
      rmSync(path, { recursive: true, force: true });
    } else if (entry.isFile() && !["package.json", "LICENSE"].includes(entry.name)) {
      rmSync(path, { force: true });
    }
  }
  const prebuilds = join(root, "prebuilds");
  for (const entry of readdirSync(prebuilds, { withFileTypes: true })) {
    if (entry.isDirectory() && entry.name !== "darwin-arm64") {
      rmSync(join(prebuilds, entry.name), { recursive: true, force: true });
    }
  }
  const library = join(root, "lib");
  for (const entry of readdirSync(library, { withFileTypes: true })) {
    if (entry.isFile() && entry.name.endsWith(".test.js")) rmSync(join(library, entry.name), { force: true });
  }
}

function trimMacosResources() {
  const hostNodeModules = join(resourcesRoot, "host", "node_modules");
  const pluginNodeModules = join(resourcesRoot, "plugins", "node_modules");
  removeDevelopmentFiles(join(resourcesRoot, "host"));
  removeDevelopmentFiles(join(resourcesRoot, "plugins"));
  reduceNodePty(hostNodeModules);
  reduceNodePty(pluginNodeModules);
  for (const relativePath of ["lucide-react", "rxjs", "xterm", "@codemirror", "@lezer", "@xterm"]) {
    rmSync(join(pluginNodeModules, relativePath), { recursive: true, force: true });
  }
}

function packageNameFromLockPath(lockPath) {
  const parts = lockPath.split("node_modules/");
  return parts.at(-1);
}

function writeLicenseManifest(packageLockPath, extras, destination) {
  const lock = readJson(packageLockPath);
  const entries = [];
  for (const [lockPath, value] of Object.entries(lock.packages ?? {})) {
    if (!lockPath || !value.version) continue;
    entries.push({
      name: packageNameFromLockPath(lockPath),
      version: value.version,
      license: value.license ?? "UNKNOWN",
      integrity: value.integrity ?? null,
    });
  }
  entries.push(...extras);
  const unique = new Map();
  for (const entry of entries) unique.set(`${entry.name}\0${entry.version}`, entry);
  writeJson(destination, [...unique.values()].sort((a, b) =>
    a.name.localeCompare(b.name) || a.version.localeCompare(b.version)));
}

function bundledNpm(nodeDistributionRoot, args, cwd, npmCache) {
  const node = join(nodeDistributionRoot, "bin", "node");
  const npmCli = join(nodeDistributionRoot, "lib", "node_modules", "npm", "bin", "npm-cli.js");
  run(node, [npmCli, ...args, "--cache", npmCache], { cwd });
}

async function stageRuntime() {
  const archivePath = join(cacheRoot, macosLock.node.archive);
  await downloadLocked(macosLock.node.url, archivePath, macosLock.node.sha256);
  const extractRoot = assertProjectPath(join(cacheRoot, "node-extracted"));
  rmSync(extractRoot, { recursive: true, force: true });
  mkdirSync(extractRoot, { recursive: true });
  run("tar", ["-xzf", archivePath, "-C", extractRoot]);
  const distributionRoot = join(extractRoot, `node-v${macosLock.node.version}-${macosLock.platform}`);
  const distributionNode = join(distributionRoot, "bin", "node");
  if (!existsSync(distributionNode) || !existsSync(join(distributionRoot, "LICENSE"))) {
    throw new Error("locked macOS Node archive is missing bin/node or LICENSE");
  }

  const nodeRoot = assertProjectPath(join(resourcesRoot, "node"));
  const hostRoot = assertProjectPath(join(resourcesRoot, "host"));
  const policyRoot = assertProjectPath(join(resourcesRoot, "policy"));
  for (const target of [nodeRoot, hostRoot, policyRoot]) {
    rmSync(target, { recursive: true, force: true });
    mkdirSync(target, { recursive: true });
  }
  copyFileSync(distributionNode, join(nodeRoot, "node"));
  chmodSync(join(nodeRoot, "node"), 0o755);
  copyFileSync(join(distributionRoot, "LICENSE"), join(nodeRoot, "LICENSE"));
  writeJson(join(nodeRoot, "runtime.lock.json"), {
    ...runtimeLock,
    node: { ...macosLock.node, platform: macosLock.platform },
  });
  copyFileSync(join(projectRoot, "runtime-policy", "dsh-market.patch.yml"), join(policyRoot, "dsh-market.patch.yml"));

  for (const name of ["package.json", "package-lock.json"]) {
    copyFileSync(join(projectRoot, "runtime-host", name), join(hostRoot, name));
  }
  bundledNpm(distributionRoot, ["ci", "--omit=dev", "--no-audit", "--fund=false"], hostRoot,
    join(cacheRoot, "npm-cache"));
  copyDirectoryContents(join(projectRoot, "runtime-host", "toolchains"), join(hostRoot, "toolchains"));
  chmodSync(join(hostRoot, "toolchains", "pnpm-10", "pnpm"), 0o755);
  copyFileSync(join(projectRoot, "runtime.lock.json"), join(hostRoot, "runtime.lock.windows.json"));
  writeJson(join(hostRoot, "runtime.lock.json"), {
    ...runtimeLock,
    node: { ...macosLock.node, platform: macosLock.platform },
  });
  copyFileSync(join(projectRoot, "THIRD_PARTY_NOTICES.md"), join(hostRoot, "THIRD_PARTY_NOTICES.md"));
  writeLicenseManifest(join(hostRoot, "package-lock.json"), [], join(hostRoot, "third-party-licenses.json"));
  return distributionRoot;
}

async function stagePlugins(nodeDistributionRoot) {
  const pluginRoot = assertProjectPath(join(resourcesRoot, "plugins"));
  rmSync(pluginRoot, { recursive: true, force: true });
  mkdirSync(pluginRoot, { recursive: true });
  for (const name of ["package.json", "package-lock.json"]) {
    copyFileSync(join(projectRoot, "plugin-runtime", name), join(pluginRoot, name));
  }
  copyFileSync(join(projectRoot, "plugins.lock.json"), join(pluginRoot, "plugins.lock.json"));
  writeFileSync(
    join(pluginRoot, "store.digest"),
    createHash("sha256").update(readFileSync(join(projectRoot, "plugins.lock.json"))).digest("hex").slice(0, 16),
    "ascii",
  );
  bundledNpm(nodeDistributionRoot,
    ["ci", "--omit=dev", "--ignore-scripts", "--legacy-peer-deps", "--no-audit", "--fund=false"],
    pluginRoot, join(cacheRoot, "plugin-npm-cache"));

  const extras = [];
  for (const plugin of pluginLock.plugins) {
    const target = join(pluginRoot, "node_modules", ...plugin.package.split("/"));
    if (plugin.source.type === "local") {
      rmSync(target, { recursive: true, force: true });
      copyDirectoryContents(join(projectRoot, plugin.source.path), target);
      extras.push({
        name: plugin.package,
        version: plugin.version,
        license: plugin.license,
        integrity: `local-${plugin.source.path}`,
      });
      continue;
    }
    if (plugin.source.type !== "github-tarball") continue;
    const archive = join(cacheRoot, "plugins", plugin.source.archive);
    await downloadLocked(plugin.source.url, archive, plugin.source.sha256);
    const safeName = plugin.package.replaceAll("@", "").replaceAll("/", "-");
    const extractRoot = assertProjectPath(join(cacheRoot, "plugins", `extract-${safeName}`));
    rmSync(extractRoot, { recursive: true, force: true });
    mkdirSync(extractRoot, { recursive: true });
    run("tar", ["-xzf", archive, "-C", extractRoot]);
    const roots = readdirSync(extractRoot, { withFileTypes: true }).filter((entry) => entry.isDirectory());
    if (roots.length !== 1) throw new Error(`plugin archive must contain one root: ${plugin.package}`);
    rmSync(target, { recursive: true, force: true });
    copyDirectoryContents(join(extractRoot, roots[0].name), target);
    extras.push({
      name: plugin.package,
      version: plugin.version,
      license: plugin.license,
      integrity: `sha256-${plugin.source.sha256}`,
    });
  }
  writeLicenseManifest(join(pluginRoot, "package-lock.json"), extras,
    join(pluginRoot, "third-party-licenses.json"));
}

async function stageMacosResources() {
  assertMacosHost();
  mkdirSync(cacheRoot, { recursive: true });
  mkdirSync(resourcesRoot, { recursive: true });
  const nodeDistributionRoot = await stageRuntime();
  await stagePlugins(nodeDistributionRoot);
  trimMacosResources();
}

function assertFile(path, description = path) {
  if (!existsSync(path) || !lstatSync(path).isFile()) throw new Error(`required file is missing: ${description}`);
}

function assertPath(path, description = path) {
  if (!existsSync(path)) throw new Error(`required path is missing: ${description}`);
}

function verifyPackageVersion(root, expectedName, expectedVersion) {
  const manifest = readJson(join(root, "package.json"));
  if (manifest.name !== expectedName || manifest.version !== expectedVersion) {
    throw new Error(`package mismatch at ${root}: ${manifest.name}@${manifest.version}`);
  }
}

function verifyLockIntegrity(lockPath, packageName, expectedVersion, expectedIntegrity) {
  const lock = readJson(lockPath);
  const entry = lock.packages?.[`node_modules/${packageName}`];
  if (entry?.version !== expectedVersion || entry?.integrity !== expectedIntegrity) {
    throw new Error(`lock entry mismatch for ${packageName}`);
  }
}

function verifyNativeModules(node, searchRoot, modules) {
  const code = `
const root = process.argv[1];
for (const name of JSON.parse(process.argv[2])) {
  const resolved = require.resolve(name, { paths: [root] });
  require(resolved);
  process.stdout.write(name + "=" + resolved + "\\n");
}`;
  run(node, ["-e", code, searchRoot, JSON.stringify(modules)]);
}

function verifyWindowsIsolation() {
  const baseConfig = readJson(join(projectRoot, "src-tauri", "tauri.conf.json"));
  if (packageJson.scripts.build !== "npm run build:payload"
      || packageJson.scripts["build:payload"] !== "pwsh -NoProfile -File scripts/build-payload.ps1"
      || JSON.stringify(baseConfig.bundle.targets) !== JSON.stringify(["nsis"])) {
    throw new Error("macOS release support must not change the Windows default build, payload script, or NSIS target");
  }
}

function verifyMacosResources() {
  assertMacosHost();
  verifyWindowsIsolation();
  const nodeRoot = join(resourcesRoot, "node");
  const hostRoot = join(resourcesRoot, "host");
  const pluginRoot = join(resourcesRoot, "plugins");
  const node = join(nodeRoot, "node");
  for (const path of [
    node,
    join(nodeRoot, "LICENSE"),
    join(nodeRoot, "runtime.lock.json"),
    join(hostRoot, runtimeLock.dsh.cliEntry),
    join(hostRoot, "toolchains", "pnpm-10", "pnpm"),
    join(hostRoot, "third-party-licenses.json"),
    join(pluginRoot, "plugins.lock.json"),
    join(pluginRoot, "third-party-licenses.json"),
    join(resourcesRoot, "policy", "dsh-market.patch.yml"),
    join(projectRoot, "src-tauri", "icons", "icon.icns"),
  ]) assertFile(path);
  if ((statSync(node).mode & 0o111) === 0) throw new Error("bundled macOS Node is not executable");
  if ((statSync(join(hostRoot, "toolchains", "pnpm-10", "pnpm")).mode & 0o111) === 0) {
    throw new Error("bundled pnpm shim is not executable");
  }
  const version = run(node, ["--version"], { capture: true }).stdout.trim().replace(/^v/, "");
  if (version !== macosLock.node.version) throw new Error(`bundled Node version mismatch: ${version}`);
  const architecture = run("file", [node], { capture: true }).stdout;
  if (!architecture.includes("arm64")) throw new Error(`bundled Node is not arm64: ${architecture.trim()}`);

  const stagedRuntimeLock = readJson(join(nodeRoot, "runtime.lock.json"));
  if (stagedRuntimeLock.node.platform !== "darwin-arm64"
      || stagedRuntimeLock.node.sha256 !== macosLock.node.sha256) {
    throw new Error("staged macOS runtime lock does not match runtime.macos.lock.json");
  }
  const dshRoot = join(hostRoot, "node_modules", "@deepseek-ai", "dsh");
  verifyPackageVersion(dshRoot, runtimeLock.dsh.package, runtimeLock.dsh.version);
  verifyPackageVersion(join(hostRoot, "node_modules", "dshmarket"), runtimeLock.market.package, runtimeLock.market.version);
  verifyPackageVersion(join(hostRoot, "node_modules", "pnpm"), runtimeLock.pnpm.package, runtimeLock.pnpm.version);
  verifyLockIntegrity(join(hostRoot, "package-lock.json"), runtimeLock.dsh.package,
    runtimeLock.dsh.version, runtimeLock.dsh.integrity);
  verifyLockIntegrity(join(hostRoot, "package-lock.json"), runtimeLock.market.package,
    runtimeLock.market.version, runtimeLock.market.integrity);
  verifyLockIntegrity(join(hostRoot, "package-lock.json"), runtimeLock.pnpm.package,
    runtimeLock.pnpm.version, runtimeLock.pnpm.integrity);
  verifyNativeModules(node, dshRoot,
    ["yaml", "@opentelemetry/api", "@aws-sdk/client-bedrock-runtime", "node-pty", "sharp"]);

  const stagedPluginLock = readJson(join(pluginRoot, "plugins.lock.json"));
  if (JSON.stringify(stagedPluginLock) !== JSON.stringify(pluginLock)) {
    throw new Error("staged plugins.lock.json differs from the tracked lock");
  }
  for (const plugin of pluginLock.plugins) {
    const root = join(pluginRoot, "node_modules", ...plugin.package.split("/"));
    verifyPackageVersion(root, plugin.package, plugin.version);
    for (const required of plugin.requiredFiles) assertPath(join(root, required), `${plugin.package}/${required}`);
    if (plugin.source.type === "npm") {
      verifyLockIntegrity(join(pluginRoot, "package-lock.json"), plugin.package,
        plugin.version, plugin.source.integrity);
    }
  }
  for (const skill of pluginLock.skills) {
    const path = join(pluginRoot, "node_modules", ...skill.sourcePackage.split("/"), skill.sourceFile);
    assertFile(path);
  }
  verifyNativeModules(node, pluginRoot, ["node-pty", "lightningcss"]);
  for (const relativePath of ["lucide-react", "rxjs", "xterm", "@codemirror", "@lezer", "@xterm"]) {
    if (existsSync(join(pluginRoot, "node_modules", relativePath))) {
      throw new Error(`duplicate plugin client dependency was not trimmed: ${relativePath}`);
    }
  }
  if (readJson(join(hostRoot, "third-party-licenses.json")).length === 0
      || readJson(join(pluginRoot, "third-party-licenses.json")).length === 0) {
    throw new Error("third-party license manifests must not be empty");
  }
}

async function treeDigest(root) {
  const digest = createHash("sha256");
  function visit(path) {
    const metadata = lstatSync(path);
    const relativePath = relative(root, path).split(sep).join("/");
    if (metadata.isSymbolicLink()) {
      digest.update(`L\0${relativePath}\0${readlinkSync(path)}\0`);
      return;
    }
    if (metadata.isDirectory()) {
      digest.update(`D\0${relativePath}\0`);
      for (const name of readdirSync(path).sort()) visit(join(path, name));
      return;
    }
    if (!metadata.isFile()) throw new Error(`unsupported resource entry: ${path}`);
    digest.update(`F\0${relativePath}\0${metadata.mode & 0o111 ? "x" : "-"}\0`);
    digest.update(readFileSync(path));
    digest.update("\0");
  }
  visit(root);
  return digest.digest("hex");
}

function directoryBytes(root) {
  let total = 0;
  function visit(path) {
    const metadata = lstatSync(path);
    if (metadata.isSymbolicLink()) return;
    if (metadata.isFile()) {
      total += metadata.size;
      return;
    }
    if (metadata.isDirectory()) for (const name of readdirSync(path)) visit(join(path, name));
  }
  visit(root);
  return total;
}

function auditProject(name, cwd, omitDev) {
  const args = ["audit", "--json"];
  if (omitDev) args.push("--omit=dev");
  const result = run("npm", args, { cwd, capture: true, allowFailure: true });
  let report;
  try {
    report = JSON.parse(result.stdout);
  } catch {
    throw new Error(`npm audit did not return JSON for ${name}: ${result.stderr.trim()}`);
  }
  const totals = report.metadata?.vulnerabilities;
  if (!totals || totals.total !== 0 || totals.high !== 0 || totals.critical !== 0) {
    throw new Error(`${name} npm audit contains advisories: ${JSON.stringify(totals)}`);
  }
  return { name, totals };
}

function runAudits() {
  const projects = [
    auditProject("desktop", projectRoot, false),
    auditProject("runtime-host", join(projectRoot, "runtime-host"), true),
    auditProject("plugin-runtime", join(projectRoot, "plugin-runtime"), true),
  ];
  writeJson(join(reportRoot, "npm-audit-macos.json"), {
    schemaVersion: 1,
    desktopVersion: packageJson.version,
    generatedAtUtc: new Date().toISOString(),
    projects,
  });
}

async function verifyBuiltArtifacts() {
  const bundleRoot = join(projectRoot, "src-tauri", "target", "release", "bundle");
  const appDirectory = join(bundleRoot, "macos");
  const dmgDirectory = join(bundleRoot, "dmg");
  const apps = existsSync(appDirectory)
    ? readdirSync(appDirectory).filter((name) => name.endsWith(".app")) : [];
  const dmgs = existsSync(dmgDirectory)
    ? readdirSync(dmgDirectory).filter((name) => name.endsWith(".dmg")) : [];
  if (apps.length !== 1 || dmgs.length !== 1) {
    throw new Error(`expected one .app and one .dmg, found ${apps.length} and ${dmgs.length}`);
  }
  const app = join(appDirectory, apps[0]);
  const dmg = join(dmgDirectory, dmgs[0]);
  const appBytes = directoryBytes(app);
  const dmgBytes = statSync(dmg).size;
  if (appBytes > macosLock.budgets.appBytes) throw new Error(`macOS app exceeds budget: ${appBytes}`);
  if (dmgBytes > macosLock.budgets.dmgBytes) throw new Error(`macOS DMG exceeds budget: ${dmgBytes}`);
  run("codesign", ["--verify", "--deep", "--strict", "--verbose=2", app]);
  run("hdiutil", ["verify", dmg]);
  const bundleNode = join(app, "Contents", "Resources", "node", "node");
  const bundleDsh = join(app, "Contents", "Resources", "host", "node_modules", "@deepseek-ai", "dsh");
  assertFile(bundleNode);
  const version = run(bundleNode, ["--version"], { capture: true }).stdout.trim().replace(/^v/, "");
  if (version !== macosLock.node.version) throw new Error(`packaged Node version mismatch: ${version}`);
  verifyNativeModules(bundleNode, bundleDsh, ["node-pty", "sharp"]);
  return {
    app,
    appBytes,
    dmg,
    dmgBytes,
    dmgSha256: await sha256File(dmg),
  };
}

async function smokeBuiltApp(app) {
  const smokeRoot = assertProjectPath(join(projectRoot, ".release-work", "macos", "smoke"));
  rmSync(smokeRoot, { recursive: true, force: true });
  const dshHome = join(smokeRoot, "dsh");
  const logDirectory = join(smokeRoot, "logs");
  mkdirSync(dshHome, { recursive: true });
  mkdirSync(logDirectory, { recursive: true });
  const executable = join(app, "Contents", "MacOS", "dsh-desktop");
  const logPath = join(logDirectory, "dsh-desktop.log");
  const environment = {
    ...process.env,
    DSH_HOME: dshHome,
    DSH_DESKTOP_LOG_DIR: logDirectory,
    DSH_DESKTOP_CORE_READY_TIMEOUT_SECS: "120",
    DSH_DESKTOP_PLUGIN_READY_TIMEOUT_SECS: "60",
  };
  const child = spawn(executable, [], { env: environment, stdio: "ignore" });
  let childExit = null;
  child.once("exit", (code, signal) => { childExit = { code, signal }; });
  const deadline = Date.now() + 120_000;
  let log = "";
  while (Date.now() < deadline) {
    if (existsSync(logPath)) log = readFileSync(logPath, "utf8");
    if (log.includes("host ready:")) break;
    if (childExit) throw new Error(`macOS smoke app exited before readiness: ${JSON.stringify(childExit)}\n${log}`);
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 500));
  }
  if (!log.includes("host ready:")) {
    child.kill("SIGKILL");
    throw new Error(`macOS smoke timed out before host readiness\n${log}`);
  }
  const hostMatch = log.match(/host started: pid=(\d+)/);
  const hostPid = hostMatch ? Number(hostMatch[1]) : null;
  run(executable, ["--quit-existing"], { env: environment });
  const exitDeadline = Date.now() + 15_000;
  while (!childExit && Date.now() < exitDeadline) {
    await new Promise((resolveDelay) => setTimeout(resolveDelay, 100));
  }
  if (!childExit || childExit.code !== 0) {
    child.kill("SIGKILL");
    throw new Error(`macOS smoke app did not exit cleanly: ${JSON.stringify(childExit)}`);
  }
  log = readFileSync(logPath, "utf8");
  if (!log.includes("secondary launch requested explicit exit")
      || !log.includes("host exited:")
      || log.includes("level=ERROR")) {
    throw new Error(`macOS smoke lifecycle evidence is incomplete\n${log}`);
  }
  if (hostPid !== null) {
    try {
      process.kill(hostPid, 0);
      throw new Error(`macOS smoke left Host pid ${hostPid} running`);
    } catch (error) {
      if (error?.code !== "ESRCH") throw error;
    }
  }
}

function buildMacosBundle() {
  const bundleRoot = assertProjectPath(join(projectRoot, "src-tauri", "target", "release", "bundle"));
  rmSync(join(bundleRoot, "macos"), { recursive: true, force: true });
  rmSync(join(bundleRoot, "dmg"), { recursive: true, force: true });
  run("npx", ["tauri", "build", "--ci", "--bundles", "app", "--config",
    "src-tauri/tauri.macos-release.conf.json"], {
    env: { MACOSX_DEPLOYMENT_TARGET: macosLock.minimumSystemVersion },
  });
  const appDirectory = join(bundleRoot, "macos");
  const apps = readdirSync(appDirectory).filter((name) => name.endsWith(".app"));
  if (apps.length !== 1) throw new Error(`expected one app before DMG creation, found ${apps.length}`);
  const dmgStaging = assertProjectPath(join(projectRoot, ".release-work", "macos", "dmg-staging"));
  rmSync(dmgStaging, { recursive: true, force: true });
  mkdirSync(dmgStaging, { recursive: true });
  cpSync(join(appDirectory, apps[0]), join(dmgStaging, apps[0]), {
    recursive: true,
    dereference: false,
    preserveTimestamps: false,
  });
  symlinkSync("/Applications", join(dmgStaging, "Applications"));
  const dmgDirectory = join(bundleRoot, "dmg");
  mkdirSync(dmgDirectory, { recursive: true });
  const dmg = join(dmgDirectory,
    `DeepSeek Harness Desktop_${packageJson.version}_aarch64.dmg`);
  run("hdiutil", ["create", "-volname", "DeepSeek Harness Desktop", "-srcfolder", dmgStaging,
    "-ov", "-format", "UDZO", dmg]);
}

async function gateMacosRelease() {
  assertMacosHost();
  rmSync(assertProjectPath(gatedRoot), { recursive: true, force: true });
  mkdirSync(reportRoot, { recursive: true });
  const phases = [];
  let passed = false;
  let failure = null;
  let artifacts = null;
  const started = Date.now();
  async function phase(name, action) {
    const phaseStarted = Date.now();
    process.stdout.write(`MACOS RELEASE GATE START: ${name}\n`);
    try {
      const value = await action();
      phases.push({ name, passed: true, durationMs: Date.now() - phaseStarted });
      process.stdout.write(`MACOS RELEASE GATE END: ${name}\n`);
      return value;
    } catch (error) {
      phases.push({ name, passed: false, durationMs: Date.now() - phaseStarted,
        failure: error instanceof Error ? error.message : String(error) });
      throw error;
    }
  }
  try {
    await phase("windowsIsolation", () => verifyWindowsIsolation());
    await phase("gitDiffCheck", () => run("git", ["diff", "--check"]));
    await phase("lint", () => run("npm", ["run", "lint"]));
    await phase("rustAndNodeTests", () => {
      run("cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "--locked"]);
      run("node", ["--test", "desktop-plugins/runtime-services/test/runtime-services.test.mjs"]);
    });
    await phase("coverage", () => run("npm", ["run", "coverage"]));
    await phase("npmAudit", () => runAudits());
    const firstDigest = await phase("resourceStageFirst", async () => {
      await stageMacosResources();
      verifyMacosResources();
      return treeDigest(resourcesRoot);
    });
    const secondDigest = await phase("resourceReproducibility", async () => {
      await stageMacosResources();
      verifyMacosResources();
      const digest = await treeDigest(resourcesRoot);
      if (digest !== firstDigest) throw new Error(`resource digest changed: ${firstDigest} != ${digest}`);
      return digest;
    });
    await phase("tauriBundle", () => buildMacosBundle());
    artifacts = await phase("artifactVerification", () => verifyBuiltArtifacts());
    await phase("installedLifecycleSmoke", () => smokeBuiltApp(artifacts.app));
    artifacts.resourceDigest = secondDigest;
    passed = true;
  } catch (error) {
    failure = error instanceof Error ? error.message : String(error);
  }

  const sourceSha = run("git", ["rev-parse", "HEAD"], { capture: true }).stdout.trim();
  const report = {
    schemaVersion: 1,
    generatedAtUtc: new Date().toISOString(),
    desktopVersion: packageJson.version,
    platform: macosLock.platform,
    sourceSha,
    nodeVersion: macosLock.node.version,
    totalMs: Date.now() - started,
    phases,
    artifact: artifacts ? {
      name: artifacts.dmg.split(sep).at(-1),
      bytes: artifacts.dmgBytes,
      sha256: artifacts.dmgSha256,
      appBytes: artifacts.appBytes,
      resourceDigest: artifacts.resourceDigest,
    } : null,
    passed,
    failure,
  };
  const reportPath = join(reportRoot, "release-gate-macos.json");
  writeJson(reportPath, report);
  if (!passed) throw new Error(`macOS release gate failed: ${failure}; report=${reportPath}`);

  const staging = assertProjectPath(`${gatedRoot}.staging.${process.pid}`);
  rmSync(staging, { recursive: true, force: true });
  mkdirSync(staging, { recursive: true });
  const artifactName = artifacts.dmg.split(sep).at(-1);
  copyFileSync(artifacts.dmg, join(staging, artifactName));
  writeFileSync(join(staging, `${artifactName}.sha256`), `${artifacts.dmgSha256}  ${artifactName}\n`, "utf8");
  copyFileSync(reportPath, join(staging, "release-gate-macos.json"));
  renameSync(staging, gatedRoot);
  process.stdout.write(`MACOS RELEASE GATE OK: artifact=${join(gatedRoot, artifactName)}\n`);
}

const command = process.argv[2];
if (command === "stage") {
  await stageMacosResources();
} else if (command === "verify") {
  verifyMacosResources();
} else if (command === "gate") {
  await gateMacosRelease();
} else {
  throw new Error("usage: node scripts/macos-release.mjs <stage|verify|gate>");
}
