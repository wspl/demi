import { test, assert, assertEq, assertThrows } from "./harness.mjs";
import * as fs from "tinyjs:fs";
import { env } from "tinyjs:runtime";

const dir = `${(env.TMPDIR ?? "/tmp").replace(/\/+$/, "")}/tinyjs-loader-${Date.now()}`;
const enc = (s) => new TextEncoder().encode(s);

test("loader: import() of absolute paths with relative imports inside", async () => {
  await fs.mkdir(`${dir}/lib`, { recursive: true });
  await fs.writeFile(`${dir}/lib/util.mjs`, enc(`export const twice = (n) => n * 2; export const where = import.meta.url;`));
  await fs.writeFile(`${dir}/entry.mjs`, enc(`import { twice, where } from "./lib/util.mjs"; export default twice(21); export { where };`));
  const mod = await import(`${dir}/entry.mjs`);
  assertEq(mod.default, 42);
  assertEq(mod.where, `file://${dir}/lib/util.mjs`);
});

test("loader: tinyjs:* is refused from a file-loaded module", async () => {
  await fs.writeFile(`${dir}/leak.mjs`, enc(`import * as fs from "tinyjs:fs"; export default fs;`));
  const e = await assertThrows(() => import(`${dir}/leak.mjs`));
  assert(/tinyjs modules are not available/.test(String(e.message ?? e)), `message: ${e.message ?? e}`);
  await fs.writeFile(`${dir}/leak2.mjs`, enc(`export default await import("tinyjs:runtime");`));
  await assertThrows(() => import(`${dir}/leak2.mjs`));
});

test("loader: the embedded bundle is refused from a file-loaded module", async () => {
  // The bundle's own modules are declared already; a file module naming one
  // must not get it back from the module cache.
  await fs.writeFile(`${dir}/leak3.mjs`, enc(`export default await import("/embedded/harness.mjs");`));
  const e = await assertThrows(() => import(`${dir}/leak3.mjs`));
  assert(/embedded bundle is not available/.test(String(e.message ?? e)), `message: ${e.message ?? e}`);
  await fs.writeFile(`${dir}/leak4.mjs`, enc(`export default await import("/x/../embedded/harness.mjs");`));
  await assertThrows(() => import(`${dir}/leak4.mjs`));
  assertEq(import.meta.url, "file:///embedded/loader.mjs");
});

test("loader: bare specifiers and missing files fail", async () => {
  await assertThrows(() => import("lodash"));
  await assertThrows(() => import(`${dir}/missing.mjs`));
  await assertThrows(() => import("tinyjs:nope"));
});

test("loader: cleanup", async () => {
  for (const f of ["lib/util.mjs", "entry.mjs", "leak.mjs", "leak2.mjs", "leak3.mjs", "leak4.mjs"]) await fs.unlink(`${dir}/${f}`);
  await fs.rmdir(`${dir}/lib`);
  await fs.rmdir(dir);
});
