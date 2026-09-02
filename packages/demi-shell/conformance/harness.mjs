// Minimal test harness: registered tests run sequentially; the process
// exit code is the verdict.
const tests = [];
export const test = (name, fn) => tests.push({ name, fn });

export const assert = (cond, message = "assertion failed") => {
  if (!cond) throw new Error(message);
};
export const assertEq = (actual, expected, message = "") => {
  const a = JSON.stringify(actual), e = JSON.stringify(expected);
  if (a !== e) throw new Error(`${message ? `${message}: ` : ""}expected ${e}, got ${a}`);
};
export const assertBytes = (actual, expected, message = "") => {
  assert(actual instanceof Uint8Array, `${message}: not a Uint8Array`);
  assertEq(Array.from(actual), Array.from(expected), message);
};
export const assertThrows = async (fn, check) => {
  try {
    await fn();
  } catch (e) {
    if (check) check(e);
    return e;
  }
  throw new Error("expected an error");
};
export const assertCode = async (fn, code, syscall) => {
  const e = await assertThrows(fn);
  assertEq(e.name, "ShellError", `error name for ${code}`);
  assertEq(e.code, code, "error code");
  if (syscall) assertEq(e.syscall, syscall, "syscall");
  assert(typeof e.errno === "number", "errno is a number");
  return e;
};
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export const run = async () => {
  let failed = 0;
  for (const { name, fn } of tests) {
    const t0 = performance.now();
    try {
      await fn();
      console.log(`ok   ${name} (${(performance.now() - t0).toFixed(1)}ms)`);
    } catch (e) {
      failed++;
      const detail = e instanceof Error ? `${e.name}: ${e.message}\n${e.stack ?? ""}` : String(e);
      console.log(`FAIL ${name}\n     ${detail.trimEnd().replace(/\n/g, "\n     ")}`);
    }
  }
  console.log(`\n${tests.length - failed}/${tests.length} passed`);
  return failed === 0 ? 0 : 1;
};
