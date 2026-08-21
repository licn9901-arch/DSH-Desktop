import { readFile, rm } from "node:fs/promises";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { pathToFileURL } from "node:url";

/** 合并 npm lockfile 中会影响运行时可达性的依赖名称。 */
function dependencyNames(pkg = {}) {
  return [
    ...Object.keys(pkg.dependencies ?? {}),
    ...Object.keys(pkg.optionalDependencies ?? {}),
    ...Object.keys(pkg.peerDependencies ?? {}),
  ];
}

/** 返回当前 npm package 路径的上一级 package 路径。 */
function parentPackagePath(packagePath) {
  const parts = packagePath.split("/");
  const index = parts.lastIndexOf("node_modules");
  return index < 0 ? "" : parts.slice(0, index).join("/");
}

/** 按 Node 的逐级 node_modules 查找规则解析 lockfile 中的依赖路径。 */
function resolveDependencyPath(packages, ownerPath, dependency) {
  let current = ownerPath;
  while (true) {
    const candidate = current
      ? `${current}/node_modules/${dependency}`
      : `node_modules/${dependency}`;
    if (Object.hasOwn(packages, candidate)) return candidate;
    if (!current) return null;
    current = parentPackagePath(current);
  }
}

/** 从一组 lockfile package 出发，收集所有运行时可达节点。 */
function collectReachable(packages, initialPaths, ignoredEdge = null) {
  const reachable = new Set();
  const pending = [...initialPaths];
  while (pending.length > 0) {
    const packagePath = pending.pop();
    if (!packagePath || reachable.has(packagePath)) continue;
    reachable.add(packagePath);
    for (const dependency of dependencyNames(packages[packagePath])) {
      if (
        ignoredEdge &&
        packagePath === ignoredEdge.ownerPath &&
        dependency === ignoredEdge.dependency
      ) {
        continue;
      }
      const resolved = resolveDependencyPath(packages, packagePath, dependency);
      if (resolved && !reachable.has(resolved)) pending.push(resolved);
    }
  }
  return reachable;
}

/**
 * 找出某个插件浏览器 bundle 已内联依赖的独占闭包。
 * 仍能从其他根依赖到达的共享 package 不会进入裁剪结果。
 */
export function findExclusiveDependencyPaths(lock, owner, dependency) {
  const packages = lock?.packages;
  if (!packages || typeof packages !== "object" || !packages[""]) {
    throw new Error("npm lockfile packages table is missing");
  }
  const rootPaths = dependencyNames(packages[""])
    .map((name) => resolveDependencyPath(packages, "", name))
    .filter(Boolean);
  const ownerPath = resolveDependencyPath(packages, "", owner);
  if (!ownerPath) throw new Error(`owner package is missing from lockfile: ${owner}`);
  if (!dependencyNames(packages[ownerPath]).includes(dependency)) {
    throw new Error(`${owner} does not declare dependency ${dependency}`);
  }
  const dependencyPath = resolveDependencyPath(packages, ownerPath, dependency);
  if (!dependencyPath) {
    throw new Error(`dependency package is missing from lockfile: ${owner} -> ${dependency}`);
  }

  const retained = collectReachable(packages, rootPaths, {
    ownerPath,
    dependency,
  });
  const bundledClosure = collectReachable(packages, [dependencyPath]);
  return [...bundledClosure].filter((path) => !retained.has(path)).sort();
}

/** 解析命令行参数并拒绝缺失值。 */
function readArgument(name) {
  const index = process.argv.indexOf(name);
  const value = index >= 0 ? process.argv[index + 1] : null;
  if (!value) throw new Error(`missing argument: ${name}`);
  return value;
}

/** 从 staging 副本删除独占依赖，所有目标必须位于指定 node_modules 内。 */
async function main() {
  const lockPath = resolve(readArgument("--lock"));
  const nodeModules = resolve(readArgument("--node-modules"));
  const owner = readArgument("--owner");
  const dependency = readArgument("--dependency");
  const lock = JSON.parse(await readFile(lockPath, "utf8"));
  const packagePaths = findExclusiveDependencyPaths(lock, owner, dependency);
  const packageRoot = dirname(nodeModules);

  for (const packagePath of [...packagePaths].sort((a, b) => b.length - a.length)) {
    const target = resolve(join(packageRoot, packagePath));
    const escaped = relative(nodeModules, target);
    if (!escaped || escaped.startsWith("..") || isAbsolute(escaped)) {
      throw new Error(`refusing to prune path outside node_modules: ${target}`);
    }
    await rm(target, { recursive: true, force: true });
  }
  console.log(
    `Plugin client dependency pruned: ${owner} -> ${dependency}, packages=${packagePaths.length}`,
  );
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  await main();
}
