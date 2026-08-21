import assert from "node:assert/strict";
import test from "node:test";

import { findExclusiveDependencyPaths } from "./prune-plugin-client-dependencies.mjs";

test("只裁剪浏览器 bundle 已内联且未被其他插件引用的依赖闭包", () => {
  const lock = {
    packages: {
      "": { dependencies: { owner: "1.0.0", consumer: "1.0.0" } },
      "node_modules/owner": {
        dependencies: { bundled: "1.0.0", shared: "1.0.0" },
      },
      "node_modules/consumer": { dependencies: { shared: "1.0.0" } },
      "node_modules/bundled": {
        dependencies: { exclusive: "1.0.0", shared: "1.0.0" },
        optionalDependencies: { optional: "1.0.0" },
      },
      "node_modules/exclusive": { peerDependencies: { exclusivePeer: "1.0.0" } },
      "node_modules/exclusivePeer": {},
      "node_modules/optional": {},
      "node_modules/shared": {},
    },
  };

  assert.deepEqual(
    findExclusiveDependencyPaths(lock, "owner", "bundled"),
    [
      "node_modules/bundled",
      "node_modules/exclusive",
      "node_modules/exclusivePeer",
      "node_modules/optional",
    ],
  );
});

test("支持 npm lockfile 中的嵌套依赖解析", () => {
  const lock = {
    packages: {
      "": { dependencies: { owner: "1.0.0" } },
      "node_modules/owner": { dependencies: { bundled: "1.0.0" } },
      "node_modules/bundled": { dependencies: { child: "2.0.0" } },
      "node_modules/bundled/node_modules/child": {},
      "node_modules/child": {},
    },
  };

  assert.deepEqual(
    findExclusiveDependencyPaths(lock, "owner", "bundled"),
    ["node_modules/bundled", "node_modules/bundled/node_modules/child"],
  );
});

test("拒绝裁剪所有者未声明的依赖", () => {
  const lock = {
    packages: {
      "": { dependencies: { owner: "1.0.0" } },
      "node_modules/owner": {},
      "node_modules/bundled": {},
    },
  };

  assert.throws(
    () => findExclusiveDependencyPaths(lock, "owner", "bundled"),
    /does not declare dependency/,
  );
});
