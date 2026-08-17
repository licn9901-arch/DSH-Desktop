import fs from 'node:fs';
import path from 'node:path';

import {
  assertFile,
  getNodeTarget,
  nodePath,
  projectRoot,
  readJson,
  run,
  sha256,
  walkDirectories,
} from './build-support.mjs';

function argument(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

const resourceRoot = path.resolve(argument('--resource-root') || path.join(projectRoot, 'src-tauri', 'resources', 'plugins'));
const lockPath = path.join(projectRoot, 'plugins.lock.json');
const lock = readJson(lockPath);
if (sha256(lockPath) !== sha256(path.join(resourceRoot, 'plugins.lock.json'))) throw new Error('Staged plugins.lock.json does not match the tracked lock file.');

const expectedOrder = [
  '@dsh-desktop/runtime-services',
  'dsh-at-file',
  '@omdsh-dev/dsh-genui',
  'dsh-better-sidebar',
  '@dsh-desktop/theme-settings',
  '@linxin666/dsh-skins',
  '@vectorize-io/hindsight-coding-agents',
  '@liustack/modlens',
  '@cubee-slide/skills-mcp-manager',
];
if (lock.plugins.map((plugin) => plugin.package).join('|') !== expectedOrder.join('|')) throw new Error('Managed plugin order is invalid.');

const themeClient = fs.readFileSync(path.join(resourceRoot, 'node_modules', '@dsh-desktop', 'theme-settings', 'lib', 'client.js'), 'utf8');
for (const marker of ['id: "desktop-theme"', '"web-ui.plugin.item"', 'renderSlot("web-ui.plugin.item"']) if (!themeClient.includes(marker)) throw new Error(`Desktop theme adapter is missing required client marker: ${marker}`);
const themeHost = fs.readFileSync(path.join(resourceRoot, 'node_modules', '@dsh-desktop', 'theme-settings', 'lib', 'index.js'), 'utf8');
for (const marker of ['/api/desktop-managed-plugins', 'PROTECTED_BUNDLES', 'atomicWriteProfile', 'serializeWrite']) if (!themeHost.includes(marker)) throw new Error(`Desktop managed-plugin API is missing required marker: ${marker}`);
const runtimeServices = fs.readFileSync(path.join(resourceRoot, 'node_modules', '@dsh-desktop', 'runtime-services', 'lib', 'index.js'), 'utf8');
for (const marker of ['ctx.provide("desktopProfiles"', 'ctx.provide("desktopPnpm"', 'unsupported profile pnpm major']) if (!runtimeServices.includes(marker)) throw new Error(`Desktop Runtime Services is missing required marker: ${marker}`);

const skillsMcpRoot = path.join(resourceRoot, 'node_modules', '@cubee-slide', 'skills-mcp-manager');
const skillsMcpClient = fs.readFileSync(path.join(skillsMcpRoot, 'lib', 'client.js'), 'utf8');
for (const forbidden of ['require("lucide-react")', "from 'lucide-react'", 'from "lucide-react"']) if (skillsMcpClient.includes(forbidden)) throw new Error(`Skills/MCP client must inline lucide-react; found external import: ${forbidden}`);
if (!fs.readFileSync(path.join(skillsMcpRoot, 'cordis.patch.yml'), 'utf8').includes('@cubee-slide/skills-mcp-manager')) throw new Error('Skills/MCP Cordis patch does not reference the published package name.');

for (const skill of lock.skills) {
  const source = path.join(resourceRoot, 'node_modules', skill.sourcePackage, skill.sourceFile);
  assertFile(source, `managed Skill ${skill.name}`);
  if (sha256(source) !== skill.sha256.toLowerCase()) throw new Error(`Managed Skill SHA-256 mismatch for ${skill.name}.`);
}

for (const plugin of lock.plugins) {
  const root = path.join(resourceRoot, 'node_modules', plugin.package);
  const manifestPath = path.join(root, 'package.json');
  assertFile(manifestPath, `plugin manifest ${plugin.package}`);
  const manifest = readJson(manifestPath);
  if (manifest.name !== plugin.package || manifest.version !== plugin.version) throw new Error(`Plugin version mismatch for ${plugin.package}: ${manifest.name} ${manifest.version}`);
  if (!manifest.dsh?.bundle?.patch) throw new Error(`Plugin bundle patch is missing from manifest: ${plugin.package}`);
  for (const required of plugin.requiredFiles) {
    const requiredPath = path.join(root, required);
    if (!fs.existsSync(requiredPath)) throw new Error(`Plugin required file is missing: ${plugin.package} / ${required}`);
  }
}

const deepseekCopies = walkDirectories(path.join(resourceRoot, 'node_modules')).filter((directory) => path.basename(directory) === '@deepseek-ai');
if (deepseekCopies.length) throw new Error(`Plugin runtime must not contain @deepseek-ai package copies: ${deepseekCopies[0]}`);

const packageLock = readJson(path.join(resourceRoot, 'package-lock.json'));
for (const plugin of lock.plugins.filter((item) => item.source.type === 'npm')) {
  const entry = packageLock.packages[`node_modules/${plugin.package}`];
  if (entry?.version !== plugin.version || entry?.integrity !== plugin.source.integrity) throw new Error(`npm lock entry does not match plugins.lock.json: ${plugin.package}`);
}
for (const dependency of lock.transitivePackages) {
  const entry = packageLock.packages[`node_modules/${dependency.package}`];
  if (entry?.version !== dependency.version || entry?.integrity !== dependency.integrity) throw new Error(`Transitive plugin lock entry mismatch: ${dependency.package}`);
}

const runtimeLock = readJson(path.join(projectRoot, 'runtime.lock.json'));
const target = getNodeTarget(runtimeLock);
const ptyBinary = path.join(resourceRoot, 'node_modules', 'node-pty', 'prebuilds', target.pty, 'pty.node');
assertFile(ptyBinary, `node-pty ${target.pty} prebuild`);
const bundledNode = nodePath(path.join(projectRoot, 'src-tauri', 'resources'), target);
if (fs.existsSync(bundledNode)) run(bundledNode, ['-e', "require(process.argv[1]); console.log('node-pty load ok')", path.join(resourceRoot, 'node_modules', 'node-pty')]);

const licenseManifest = readJson(path.join(resourceRoot, 'third-party-licenses.json'));
if (!Array.isArray(licenseManifest) || licenseManifest.length === 0) throw new Error('Plugin third-party license manifest is empty.');
const licensedNames = new Set(licenseManifest.map((entry) => entry.name));
for (const plugin of lock.plugins) if (!licensedNames.has(plugin.package)) throw new Error(`Plugin is missing from third-party license manifest: ${plugin.package}`);

console.log(`Plugins valid: ${lock.plugins.length} managed bundles, ${lock.skills.length} managed Skills, PTY ${target.pty}, ${licenseManifest.length} licensed packages.`);
