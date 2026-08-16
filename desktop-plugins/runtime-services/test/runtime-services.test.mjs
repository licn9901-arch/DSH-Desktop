import assert from "node:assert/strict";
import { test } from "node:test";
import { selectPnpmMajor } from "../lib/index.js";

test("从 modules 元数据选择固定 pnpm major", () => {
  assert.equal(selectPnpmMajor("packageManager: pnpm@9.15.9\n"), 9);
  assert.equal(selectPnpmMajor("packageManager: 'pnpm@10.33.2+sha512-test'\n"), 10);
  assert.equal(selectPnpmMajor("packageManager: pnpm@11.22.0\n"), 11);
  assert.equal(selectPnpmMajor(""), 11);
});

test("拒绝没有固定运行时的 pnpm major", () => {
  assert.throws(() => selectPnpmMajor("packageManager: pnpm@12.0.0\n"), /unsupported profile pnpm major/);
});
