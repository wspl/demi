import { test, assert, assertEq, assertCode } from "./harness.mjs";
import * as rt from "demishell:runtime";
import * as fs from "demishell:fs";

test("runtime: argv/env/pid/identity/version", () => {
  assert(Array.isArray(rt.argv) && rt.argv.length >= 1, "argv");
  assert(typeof rt.env === "object" && rt.env !== null, "env");
  assert(Object.isFrozen(rt.env), "env frozen");
  assert(rt.pid > 0, "pid");
  assertEq([rt.stdin, rt.stdout, rt.stderr], [0, 1, 2]);
  assert(typeof rt.identity.uid === "number" && typeof rt.identity.hostname === "string" && rt.identity.homeDir.startsWith("/"), "identity");
  assertEq([rt.version, rt.abi], [1, 1]);
  assert(typeof rt.openHandles() === "number", "openHandles");
});

test("runtime: cwd/chdir", async () => {
  const before = rt.cwd();
  rt.chdir("/");
  assertEq(rt.cwd(), "/");
  rt.chdir(before);
  assertEq(rt.cwd(), before);
  await assertCode(() => rt.chdir("/nonexistent-dir-xyz"), "ENOENT", "chdir");
});

test("runtime: stdout via fs.write", async () => {
  await fs.write(rt.stdout, new TextEncoder().encode("     (written through fd 1)\n"));
});

test("runtime: onSignal registers a handler", async () => {
  let got = null;
  rt.onSignal("SIGUSR2", (name) => (got = name));
  // The signal is raised from a child in the process suite; here only the
  // registration and the unsupported-name error are checked.
  await assertCode(() => rt.onSignal("SIGKILL", () => {}), "EINVAL", "signal");
  assertEq(got, null);
});
