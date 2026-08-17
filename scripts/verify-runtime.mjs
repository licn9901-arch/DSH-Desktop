import fs from 'node:fs';
import path from 'node:path';

import {
  assertFile,
  getNodeTarget,
  nodePath,
  projectRoot,
  readJson,
  runCapture,
  sha256,
} from './build-support.mjs';

function argument(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

const resourceRoot = path.resolve(argument('--resource-root') || path.join(projectRoot, 'src-tauri', 'resources'));
const archivePath = argument('--archive-path');
const runtimeLock = readJson(path.join(projectRoot, 'runtime.lock.json'));
const target = getNodeTarget(runtimeLock);
const nodeRoot = path.join(resourceRoot, 'node');
const hostRoot = path.join(resourceRoot, 'host');
const nodeExecutable = nodePath(resourceRoot, target);
const cliPath = path.join(hostRoot, runtimeLock.dsh.cliEntry);
const dshRoot = path.join(hostRoot, 'node_modules', '@deepseek-ai', 'dsh');
const marketRoot = path.join(hostRoot, 'node_modules', 'dshmarket');
const pnpmRoot = path.join(hostRoot, 'node_modules', 'pnpm');
const policyPath = path.join(resourceRoot, 'policy', 'dsh-market.patch.yml');
const webCandidates = [
  path.join(hostRoot, 'node_modules', '@deepseek-ai', 'dsh-web-frontend'),
  path.join(dshRoot, 'node_modules', '@deepseek-ai', 'dsh-web-frontend'),
];
const webRoot = webCandidates.find((candidate) => fs.existsSync(candidate));
if (!webRoot) throw new Error(`Bundled Web frontend package is missing. Checked: ${webCandidates.join('; ')}`);

for (const file of [
  nodeExecutable,
  path.join(nodeRoot, 'LICENSE'),
  path.join(nodeRoot, 'runtime.lock.json'),
  cliPath,
  path.join(dshRoot, 'package.json'),
  path.join(dshRoot, 'LICENSE'),
  path.join(marketRoot, 'package.json'),
  path.join(marketRoot, 'LICENSE'),
  path.join(marketRoot, 'cordis.patch.yml'),
  path.join(pnpmRoot, 'package.json'),
  path.join(pnpmRoot, 'LICENSE'),
  path.join(pnpmRoot, 'bin', 'pnpm.mjs'),
  path.join(hostRoot, 'node_modules', '.bin', process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm'),
  path.join(hostRoot, 'toolchains', 'pnpm-9', process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm'),
  path.join(hostRoot, 'toolchains', 'pnpm-10', process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm'),
  path.join(hostRoot, 'toolchains', 'pnpm-11', process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm'),
  policyPath,
  path.join(webRoot, 'dist', 'index.html'),
  path.join(webRoot, 'LICENSE'),
  path.join(hostRoot, 'package-lock.json'),
  path.join(hostRoot, 'THIRD_PARTY_NOTICES.md'),
  path.join(hostRoot, 'third-party-licenses.json'),
]) assertFile(file);

const rootLockHash = sha256(path.join(projectRoot, 'runtime.lock.json'));
for (const stagedLock of [path.join(nodeRoot, 'runtime.lock.json'), path.join(hostRoot, 'runtime.lock.json')]) {
  if (sha256(stagedLock) !== rootLockHash) throw new Error(`Staged runtime lock is stale: ${stagedLock}`);
}
if (archivePath && sha256(path.resolve(archivePath)) !== target.sha256.toLowerCase()) {
  throw new Error(`Cached Node archive SHA-256 mismatch: ${archivePath}`);
}

const nodeVersion = runCapture(nodeExecutable, ['--version']).stdout.trim().replace(/^v/, '');
if (nodeVersion !== runtimeLock.node.version) throw new Error(`Bundled Node version mismatch. Expected ${runtimeLock.node.version}, got ${nodeVersion}.`);

const dshPackage = readJson(path.join(dshRoot, 'package.json'));
if (dshPackage.name !== runtimeLock.dsh.package || dshPackage.version !== runtimeLock.dsh.version) throw new Error('Bundled DSH package mismatch.');
const hostLock = readJson(path.join(hostRoot, 'package-lock.json'));
const lockedDsh = hostLock.packages['node_modules/@deepseek-ai/dsh'];
if (lockedDsh?.version !== runtimeLock.dsh.version || lockedDsh?.integrity !== runtimeLock.dsh.integrity) throw new Error('Bundled DSH lock entry does not match runtime.lock.json.');

const marketPackage = readJson(path.join(marketRoot, 'package.json'));
const lockedMarket = hostLock.packages['node_modules/dshmarket'];
if (marketPackage.name !== runtimeLock.market.package || marketPackage.version !== runtimeLock.market.version || lockedMarket?.version !== runtimeLock.market.version || lockedMarket?.integrity !== runtimeLock.market.integrity) throw new Error('Bundled DSH Market does not match runtime.lock.json.');
const sourceClient = fs.readFileSync(path.join(marketRoot, 'client', 'client.js'), 'utf8');
if (!sourceClient.startsWith('window.__ModuleLoader__.load({ id: "dshmarket", factory:')) throw new Error('Bundled Market client must keep the upstream dshmarket registration ID.');

const pnpmPackage = readJson(path.join(pnpmRoot, 'package.json'));
const lockedPnpm = hostLock.packages['node_modules/pnpm'];
if (pnpmPackage.name !== runtimeLock.pnpm.package || pnpmPackage.version !== runtimeLock.pnpm.version || lockedPnpm?.version !== runtimeLock.pnpm.version || lockedPnpm?.integrity !== runtimeLock.pnpm.integrity) throw new Error('Bundled pnpm does not match runtime.lock.json.');
if (pnpmPackage.engines.node !== runtimeLock.pnpm.nodeRange) throw new Error(`Bundled pnpm Node compatibility mismatch: ${pnpmPackage.engines.node}`);
for (const toolchain of runtimeLock.pnpmToolchains) {
  const packagePath = path.join(hostRoot, 'node_modules', toolchain.package, 'package.json');
  const packageJson = readJson(packagePath);
  const entry = hostLock.packages[`node_modules/${toolchain.package}`];
  if (packageJson.version !== toolchain.version || entry?.version !== toolchain.version || entry?.integrity !== toolchain.integrity) throw new Error(`Bundled ${toolchain.package} does not match runtime.lock.json.`);
}

const policy = fs.readFileSync(policyPath, 'utf8');
if (!/^- id:\s*dsh-market\s*$/m.test(policy) || !/^\s*profile:\s*web\s*$/m.test(policy) || !/^\s*allowRestart:\s*false\s*$/m.test(policy)) throw new Error('Desktop DSH Market policy must configure profile=web and allowRestart=false.');
const webPackage = readJson(path.join(webRoot, 'package.json'));
if (webPackage.version !== runtimeLock.dsh.version) throw new Error(`Bundled Web frontend version mismatch: ${webPackage.version}`);
const licenseEntries = readJson(path.join(hostRoot, 'third-party-licenses.json'));
if (!Array.isArray(licenseEntries) || licenseEntries.length === 0) throw new Error('Third-party license manifest is empty.');

console.log(`Runtime valid: Node ${nodeVersion}, DSH ${dshPackage.version}, Market ${marketPackage.version}, pnpm ${pnpmPackage.version}, ${licenseEntries.length} licensed package entries, target ${target.key}.`);
