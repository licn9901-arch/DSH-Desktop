import fs from 'node:fs';
import path from 'node:path';

import {
  assertDirectory,
  copyTree,
  download,
  ensureDirectory,
  extractArchive,
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
const lockPath = path.join(projectRoot, 'plugins.lock.json');
const lock = readJson(lockPath);
const cacheRoot = path.join(projectRoot, '.runtime-cache', 'plugins');
const npmCache = process.env.DSH_DESKTOP_NPM_CACHE || path.join(projectRoot, '.runtime-cache', 'plugin-npm-cache');
const resourceRoot = path.join(projectRoot, 'src-tauri', 'resources', 'plugins');

ensureDirectory(cacheRoot);
ensureDirectory(npmCache, { projectBound: false });
removeDirectory(resourceRoot);
ensureDirectory(resourceRoot);
fs.copyFileSync(lockPath, path.join(resourceRoot, 'plugins.lock.json'));
for (const file of ['package.json', 'package-lock.json']) {
  fs.copyFileSync(path.join(projectRoot, 'plugin-runtime', file), path.join(resourceRoot, file));
}

console.log('Installing locked npm plugin dependencies with lifecycle scripts disabled...');
run(npmCommand(), [
  'ci', '--omit=dev', '--ignore-scripts', '--legacy-peer-deps', '--no-audit', '--fund=false',
  '--cache', npmCache, ...(offline ? ['--offline'] : []),
], { cwd: resourceRoot });

for (const plugin of lock.plugins) {
  const target = path.join(resourceRoot, 'node_modules', plugin.package);
  ensureDirectory(target);
  if (plugin.source.type === 'local') {
    const source = path.join(projectRoot, plugin.source.path);
    assertDirectory(source, `local plugin source ${plugin.package}`);
    copyTree(source, target);
    continue;
  }
  if (!['github-tarball', 'github-release-asset'].includes(plugin.source.type)) continue;

  const archivePath = path.join(cacheRoot, plugin.source.archive);
  const researchCache = path.join(projectRoot, '.runtime-cache', 'plugin-research', plugin.source.archive);
  if (!fs.existsSync(archivePath) && fs.existsSync(researchCache)) fs.copyFileSync(researchCache, archivePath);
  if (!fs.existsSync(archivePath)) {
    if (offline) throw new Error(`Offline staging requires cached archive: ${archivePath}`);
    console.log(`Downloading ${plugin.package} ${plugin.version}...`);
    await download(plugin.source.url, archivePath);
  }
  const actual = sha256(archivePath);
  if (actual !== plugin.source.sha256.toLowerCase()) {
    throw new Error(`Archive SHA-256 mismatch for ${plugin.package}. Expected ${plugin.source.sha256}, got ${actual}.`);
  }
  const extractRoot = path.join(cacheRoot, `extract-${plugin.package.replaceAll('@', '').replaceAll('/', '-')}`);
  removeDirectory(extractRoot);
  ensureDirectory(extractRoot);
  extractArchive(archivePath, extractRoot);
  copyTree(singleDirectory(extractRoot), target);
}

const packageLicenses = packageLicenseEntries(readJson(path.join(resourceRoot, 'package-lock.json')));
const lockedLicenses = lock.plugins.map((plugin) => ({
  name: plugin.package,
  version: plugin.version,
  license: plugin.license,
  integrity: plugin.source.type === 'local' ? `local-${plugin.source.path}` : `sha256-${plugin.source.sha256}`,
}));
writeJson(path.join(resourceRoot, 'third-party-licenses.json'), [...packageLicenses, ...lockedLicenses].sort((a, b) => `${a.name}@${a.version}`.localeCompare(`${b.name}@${b.version}`)));

run(process.execPath, [path.join(projectRoot, 'scripts', 'verify-plugins.mjs'), '--resource-root', resourceRoot]);
console.log(`Managed plugins staged at ${resourceRoot}.`);
