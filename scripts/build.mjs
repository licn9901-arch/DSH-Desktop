import fs from 'node:fs';
import path from 'node:path';

import { projectRoot, removeDirectory, run } from './build-support.mjs';

const npmCache = process.env.DSH_DESKTOP_NPM_CACHE;
const commandEnvironment = npmCache ? { ...process.env, DSH_DESKTOP_NPM_CACHE: npmCache } : process.env;

run(process.execPath, [path.join(projectRoot, 'scripts', 'validate-icons.mjs')], { env: commandEnvironment });
run(process.execPath, [path.join(projectRoot, 'scripts', 'stage-runtime.mjs')], { env: commandEnvironment });
run(process.execPath, [path.join(projectRoot, 'scripts', 'verify-runtime.mjs')], { env: commandEnvironment });
run(process.execPath, [path.join(projectRoot, 'scripts', 'stage-plugins.mjs')], { env: commandEnvironment });
run(process.execPath, [path.join(projectRoot, 'scripts', 'verify-plugins.mjs')], { env: commandEnvironment });

const tauri = path.join(projectRoot, 'node_modules', '.bin', process.platform === 'win32' ? 'tauri.cmd' : 'tauri');
if (process.platform === 'darwin') {
  run(tauri, ['build', '--bundles', 'app'], { env: commandEnvironment });

  // create-dmg's Finder prettifying AppleScript needs an interactive Finder
  // session. Build a deterministic, drag-and-drop DMG for headless CI and SSH
  // sessions instead; the .app itself remains the source of truth.
  const appDirectory = path.join(projectRoot, 'src-tauri', 'target', 'release', 'bundle', 'macos');
  const appName = 'DeepSeek Harness Desktop.app';
  const appPath = path.join(appDirectory, appName);
  const dmgDirectory = path.join(projectRoot, 'src-tauri', 'target', 'release', 'bundle', 'dmg');
  fs.mkdirSync(dmgDirectory, { recursive: true });
  const version = JSON.parse(fs.readFileSync(path.join(projectRoot, 'package.json'), 'utf8')).version;
  const architecture = process.arch === 'arm64' ? 'aarch64' : 'x64';
  const dmgPath = path.join(dmgDirectory, `DeepSeek Harness Desktop_${version}_${architecture}.dmg`);
  const sourceDirectory = fs.mkdtempSync(path.join(projectRoot, '.runtime-cache', 'dmg-source-'));
  const stagedAppPath = path.join(sourceDirectory, appName);
  try {
    fs.cpSync(appPath, stagedAppPath, { recursive: true, force: true, dereference: false });
    run('hdiutil', ['create', '-volname', 'DeepSeek Harness Desktop', '-srcfolder', sourceDirectory, '-ov', '-format', 'UDZO', dmgPath], { env: commandEnvironment });
  } finally {
    removeDirectory(sourceDirectory);
  }
  console.log(`macOS app bundle: ${appPath}`);
  console.log(`macOS DMG: ${dmgPath}`);
} else {
  run(tauri, ['build'], { env: commandEnvironment });
}
