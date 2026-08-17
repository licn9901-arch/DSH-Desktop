import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

export const projectRoot = path.resolve(import.meta.dirname, '..');

export function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, 'utf8'));
}

export function writeJson(filePath, value) {
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`);
}

export function assertInsideProject(filePath) {
  const resolved = path.resolve(filePath);
  const relative = path.relative(projectRoot, resolved);
  if (relative === '' || relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
    throw new Error(`Refusing to modify a path outside the project: ${resolved}`);
  }
  return resolved;
}

export function removeDirectory(directory) {
  if (fs.existsSync(directory)) {
    fs.rmSync(assertInsideProject(directory), { recursive: true, force: true });
  }
}

export function ensureDirectory(directory, options = {}) {
  const target = options.projectBound === false ? path.resolve(directory) : assertInsideProject(directory);
  fs.mkdirSync(target, { recursive: true });
}

export function copyTree(source, destination) {
  ensureDirectory(path.dirname(destination));
  fs.cpSync(source, destination, { recursive: true, force: true, dereference: false });
}

export function sha256(filePath) {
  return crypto.createHash('sha256').update(fs.readFileSync(filePath)).digest('hex');
}

export function getNodeTarget(runtimeLock) {
  const requested = process.env.DSH_DESKTOP_NODE_TARGET;
  const architecture = process.arch === 'arm64' ? 'arm64' : process.arch === 'x64' ? 'x64' : null;
  const detected = process.platform === 'win32' && architecture === 'x64'
    ? 'win-x64'
    : process.platform === 'darwin' && architecture
      ? `darwin-${architecture}`
      : null;
  const key = requested || detected;
  if (!key || !runtimeLock.node.targets?.[key]) {
    throw new Error(
      `Unsupported Node runtime target ${key || `${process.platform}-${process.arch}`}. ` +
      `Set DSH_DESKTOP_NODE_TARGET to one of: ${Object.keys(runtimeLock.node.targets || {}).join(', ')}`,
    );
  }
  return { key, ...runtimeLock.node.targets[key] };
}

export function nodePath(resourceRoot, target) {
  return path.join(resourceRoot, 'node', target.executable);
}

export function npmCommand() {
  return process.platform === 'win32' ? 'npm.cmd' : 'npm';
}

export function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd || projectRoot,
    env: options.env || process.env,
    stdio: options.stdio || 'inherit',
    encoding: 'utf8',
  });
  if (result.error) {
    throw new Error(`Failed to start ${command}: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(`${command} exited with status ${result.status}`);
  }
  return result;
}

export function runCapture(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd || projectRoot,
    env: options.env || process.env,
    encoding: 'utf8',
  });
  if (result.error) {
    throw new Error(`Failed to start ${command}: ${result.error.message}`);
  }
  return result;
}

export async function download(url, destination) {
  // curl respects the proxy settings used by the desktop build environment and
  // is available on macOS, modern Windows, and the GitHub runner images.
  run(process.platform === 'win32' ? 'curl.exe' : 'curl', [
    '--fail', '--location', '--retry', '3', '--retry-all-errors', '--connect-timeout', '30',
    '--output', destination, url,
  ]);
}

export function extractArchive(archivePath, destination) {
  ensureDirectory(destination);
  const gzip = archivePath.endsWith('.tar.gz') || archivePath.endsWith('.tgz');
  const args = gzip
    ? ['-xzf', archivePath, '-C', destination]
    : ['-xf', archivePath, '-C', destination];
  run(process.platform === 'win32' ? 'tar.exe' : 'tar', args);
}

export function singleDirectory(directory) {
  const entries = fs.readdirSync(directory, { withFileTypes: true }).filter((entry) => entry.isDirectory());
  if (entries.length !== 1) {
    throw new Error(`Archive must contain exactly one root directory: ${directory}`);
  }
  return path.join(directory, entries[0].name);
}

export function packageLicenseEntries(packageLock) {
  return Object.entries(packageLock.packages || {})
    .filter(([name, value]) => name && value?.version)
    .map(([name, value]) => ({
      name: name.split('node_modules/').at(-1),
      version: value.version,
      license: value.license || 'UNKNOWN',
      integrity: value.integrity,
    }))
    .sort((a, b) => `${a.name}@${a.version}`.localeCompare(`${b.name}@${b.version}`));
}

export function ptyPlatform(target) {
  return target.pty;
}

export function walkDirectories(root) {
  const found = [];
  if (!fs.existsSync(root)) return found;
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    const fullPath = path.join(root, entry.name);
    if (entry.isDirectory() && !entry.isSymbolicLink()) {
      found.push(fullPath, ...walkDirectories(fullPath));
    }
  }
  return found;
}

export function assertFile(filePath, description = filePath) {
  if (!fs.existsSync(filePath) || !fs.statSync(filePath).isFile()) {
    throw new Error(`Required file is missing: ${description}: ${filePath}`);
  }
}

export function assertDirectory(directory, description = directory) {
  if (!fs.existsSync(directory) || !fs.statSync(directory).isDirectory()) {
    throw new Error(`Required directory is missing: ${description}: ${directory}`);
  }
}
