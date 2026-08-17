import fs from 'node:fs';
import path from 'node:path';

import {
  assertFile,
  copyTree,
  download,
  ensureDirectory,
  extractArchive,
  getNodeTarget,
  nodePath,
  npmCommand,
  packageLicenseEntries,
  projectRoot,
  readJson,
  removeDirectory,
  run,
  sha256,
  singleDirectory,
  writeJson,
} from './build-support.mjs';

const offline = process.argv.includes('--offline');
const runtimeLockPath = path.join(projectRoot, 'runtime.lock.json');
const runtimeLock = readJson(runtimeLockPath);
const target = getNodeTarget(runtimeLock);
const cacheRoot = path.join(projectRoot, '.runtime-cache');
const archivePath = path.join(cacheRoot, target.archive);
const resourceRoot = path.join(projectRoot, 'src-tauri', 'resources');
const hostResourceRoot = path.join(resourceRoot, 'host');
const npmCache = process.env.DSH_DESKTOP_NPM_CACHE || path.join(cacheRoot, 'npm-cache');

ensureDirectory(cacheRoot);
ensureDirectory(npmCache, { projectBound: false });
if (!fs.existsSync(archivePath)) {
  if (offline) throw new Error(`Offline staging requires the cached Node archive: ${archivePath}`);
  console.log(`Downloading Node.js ${runtimeLock.node.version} for ${target.key}...`);
  await download(target.url, archivePath);
}
if (sha256(archivePath) !== target.sha256.toLowerCase()) {
  throw new Error(`Node archive SHA-256 mismatch. Expected ${target.sha256}, got ${sha256(archivePath)}.`);
}

const extractRoot = path.join(cacheRoot, `node-extracted-${target.key}`);
removeDirectory(extractRoot);
ensureDirectory(extractRoot);
extractArchive(archivePath, extractRoot);
const distributionRoot = path.join(extractRoot, `node-v${runtimeLock.node.version}-${target.key}`);
const distributionNode = path.join(distributionRoot, target.executable);
assertFile(distributionNode, `Node executable for ${target.key}`);
assertFile(path.join(distributionRoot, 'LICENSE'), 'Node license');

for (const directory of ['node', 'host', 'policy']) removeDirectory(path.join(resourceRoot, directory));
for (const directory of ['node', 'host', 'policy']) ensureDirectory(path.join(resourceRoot, directory));

const stagedNode = nodePath(resourceRoot, target);
ensureDirectory(path.dirname(stagedNode));
fs.copyFileSync(distributionNode, stagedNode);
fs.chmodSync(stagedNode, 0o755);
fs.copyFileSync(path.join(distributionRoot, 'LICENSE'), path.join(resourceRoot, 'node', 'LICENSE'));
fs.copyFileSync(runtimeLockPath, path.join(resourceRoot, 'node', 'runtime.lock.json'));
fs.copyFileSync(
  path.join(projectRoot, 'runtime-policy', 'dsh-market.patch.yml'),
  path.join(resourceRoot, 'policy', 'dsh-market.patch.yml'),
);

for (const file of ['package.json', 'package-lock.json']) {
  fs.copyFileSync(path.join(projectRoot, 'runtime-host', file), path.join(hostResourceRoot, file));
}
console.log('Installing DSH, Market and pnpm with npm ci...');
run(npmCommand(), ['ci', '--omit=dev', '--no-audit', '--fund=false', '--cache', npmCache, ...(offline ? ['--offline'] : [])], {
  cwd: hostResourceRoot,
});
copyTree(path.join(projectRoot, 'runtime-host', 'toolchains'), path.join(hostResourceRoot, 'toolchains'));
fs.copyFileSync(runtimeLockPath, path.join(hostResourceRoot, 'runtime.lock.json'));
fs.copyFileSync(path.join(projectRoot, 'THIRD_PARTY_NOTICES.md'), path.join(hostResourceRoot, 'THIRD_PARTY_NOTICES.md'));
writeJson(path.join(hostResourceRoot, 'third-party-licenses.json'), packageLicenseEntries(readJson(path.join(hostResourceRoot, 'package-lock.json'))));

run(process.execPath, [path.join(projectRoot, 'scripts', 'verify-runtime.mjs'), '--resource-root', resourceRoot, '--archive-path', archivePath]);
console.log(`Self-contained runtime staged at ${resourceRoot} for ${target.key}.`);
