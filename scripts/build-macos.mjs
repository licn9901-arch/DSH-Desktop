import { createHash } from 'node:crypto'
import { chmod, copyFile, cp, mkdir, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises'
import { arch, platform } from 'node:os'
import { dirname, join, resolve, sep } from 'node:path'
import { spawn } from 'node:child_process'

const root = resolve(import.meta.dirname, '..')
const resources = join(root, 'src-tauri', 'resources')
const cache = join(root, '.runtime-cache')

function insideRoot(path) {
  const resolved = resolve(path)
  if (resolved !== root && !resolved.startsWith(`${root}${sep}`)) throw new Error(`Refusing path outside repository: ${resolved}`)
  return resolved
}

function run(command, args, options = {}) {
  return new Promise((ok, fail) => {
    const child = spawn(command, args, { cwd: root, stdio: 'inherit', ...options })
    child.on('error', fail)
    child.on('exit', code => code === 0 ? ok() : fail(new Error(`${command} exited with ${code}`)))
  })
}

async function json(path) { return JSON.parse(await readFile(path, 'utf8')) }
async function exists(path) { try { return (await stat(path)).isFile() } catch { return false } }
async function pathExists(path) { try { await stat(path); return true } catch { return false } }
async function sha256(path) { return createHash('sha256').update(await readFile(path)).digest('hex') }

async function download(url, target) {
  if (await exists(target)) return
  const response = await fetch(url, { headers: { 'user-agent': 'DSH-Desktop-build' } })
  if (!response.ok) throw new Error(`Download failed (${response.status}): ${url}`)
  await mkdir(dirname(target), { recursive: true })
  await writeFile(target, new Uint8Array(await response.arrayBuffer()))
}

async function replaceDir(path) {
  insideRoot(path)
  await rm(path, { recursive: true, force: true })
  await mkdir(path, { recursive: true })
}

async function stageRuntime() {
  const lockPath = join(root, 'runtime.lock.json')
  const lock = await json(lockPath)
  const cpu = arch() === 'arm64' ? 'arm64' : 'x64'
  const node = lock.node.darwin[cpu]
  const archive = join(cache, node.archive)
  await download(node.url, archive)
  const actual = await sha256(archive)
  if (actual !== node.sha256) throw new Error(`Node archive SHA-256 mismatch: ${actual}`)

  const extracted = join(cache, `node-darwin-${cpu}`)
  await replaceDir(extracted)
  await run('tar', ['-xzf', archive, '-C', extracted])
  const distribution = join(extracted, `node-v${lock.node.version}-darwin-${cpu}`)
  const nodeRoot = join(resources, 'node')
  const hostRoot = join(resources, 'host')
  const policyRoot = join(resources, 'policy')
  for (const path of [nodeRoot, hostRoot, policyRoot]) await replaceDir(path)
  await copyFile(join(distribution, 'bin', 'node'), join(nodeRoot, 'node'))
  await chmod(join(nodeRoot, 'node'), 0o755)
  await copyFile(join(distribution, 'LICENSE'), join(nodeRoot, 'LICENSE'))
  await copyFile(lockPath, join(nodeRoot, 'runtime.lock.json'))
  await copyFile(join(root, 'runtime-policy', 'dsh-market.patch.yml'), join(policyRoot, 'dsh-market.patch.yml'))
  for (const file of ['package.json', 'package-lock.json']) await copyFile(join(root, 'runtime-host', file), join(hostRoot, file))
  await run('npm', ['ci', '--omit=dev', '--no-audit', '--fund=false', '--cache', join(cache, 'npm-cache')], { cwd: hostRoot })
  for (const file of ['runtime.lock.json', 'THIRD_PARTY_NOTICES.md']) await copyFile(join(root, file), join(hostRoot, file))
  const hostLock = await json(join(hostRoot, 'package-lock.json'))
  const licenses = Object.entries(hostLock.packages).filter(([, value]) => value.version).map(([path, value]) => ({
    name: path.split('node_modules/').at(-1), version: value.version, license: value.license || 'UNKNOWN', integrity: value.integrity
  })).sort((a, b) => a.name.localeCompare(b.name))
  await writeFile(join(hostRoot, 'third-party-licenses.json'), `${JSON.stringify(licenses, null, 2)}\n`)
  const version = (await new Promise((ok, fail) => {
    let output = ''; const child = spawn(join(nodeRoot, 'node'), ['--version']); child.stdout.on('data', d => output += d); child.on('error', fail); child.on('exit', c => c ? fail(new Error('Bundled Node failed')) : ok(output.trim()))
  }))
  if (version !== `v${lock.node.version}`) throw new Error(`Bundled Node version mismatch: ${version}`)
}

async function stagePlugins() {
  const lockPath = join(root, 'plugins.lock.json')
  const lock = await json(lockPath)
  const target = join(resources, 'plugins')
  await replaceDir(target)
  for (const file of ['package.json', 'package-lock.json']) await copyFile(join(root, 'plugin-runtime', file), join(target, file))
  await copyFile(lockPath, join(target, 'plugins.lock.json'))
  await run('npm', ['ci', '--omit=dev', '--ignore-scripts', '--legacy-peer-deps', '--no-audit', '--fund=false', '--cache', join(cache, 'plugin-npm-cache')], { cwd: target })
  for (const plugin of lock.plugins) {
    const destination = join(target, 'node_modules', ...plugin.package.split('/'))
    if (plugin.source.type === 'local') {
      await mkdir(destination, { recursive: true }); await cp(join(root, plugin.source.path), destination, { recursive: true })
    } else if (plugin.source.type === 'github-tarball') {
      const archive = join(cache, 'plugins', plugin.source.archive)
      await download(plugin.source.url, archive)
      const actual = await sha256(archive)
      if (actual !== plugin.source.sha256) throw new Error(`${plugin.package} archive SHA-256 mismatch: ${actual}`)
      const extracted = join(cache, 'plugins', `extract-${plugin.package.replaceAll('/', '-').replace('@', '')}`)
      await replaceDir(extracted); await run('tar', ['-xzf', archive, '-C', extracted])
      const roots = await readdir(extracted)
      if (roots.length !== 1) throw new Error(`${plugin.package} archive must contain one root`)
      await mkdir(destination, { recursive: true }); await cp(join(extracted, roots[0]), destination, { recursive: true })
    }
    for (const file of plugin.requiredFiles) if (!(await pathExists(join(destination, file)))) throw new Error(`${plugin.package} is missing ${file}`)
  }
  const pty = join(target, 'node_modules', 'node-pty', 'prebuilds', `darwin-${arch()}`, 'pty.node')
  if (!(await exists(pty))) throw new Error(`Better Sidebar macOS ${arch()} PTY prebuild is missing`)
  const licenses = lock.plugins.map(p => ({ name: p.package, version: p.version, license: p.license, integrity: p.source.integrity || `locked-${p.source.type}` }))
  await writeFile(join(target, 'third-party-licenses.json'), `${JSON.stringify(licenses, null, 2)}\n`)
}

if (platform() !== 'darwin') throw new Error('build:macos must run on macOS')
await run('npm', ['ci'])
if (!(await exists(join(root, 'src-tauri', 'icons', 'icon.icns')))) {
  await run('npx', ['tauri', 'icon', 'src-tauri/icons/icon.png'])
}
await stageRuntime()
await stagePlugins()
await run('npx', ['tauri', 'build', '--bundles', 'dmg'], { env: { ...process.env, CI: 'true' } })
console.log('DMG written under src-tauri/target/release/bundle/dmg')
