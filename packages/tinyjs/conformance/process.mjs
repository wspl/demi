import { test, assert, assertEq, assertCode, sleep } from "./harness.mjs";
import * as proc from "tinyjs:process";
import * as fs from "tinyjs:fs";
import { env, pid as ownPid, onSignal, identity } from "tinyjs:runtime";
const ownUid = identity.uid;

const enc = (s) => new TextEncoder().encode(s);
const dec = (b) => new TextDecoder().decode(b);
const baseEnv = { PATH: env.PATH ?? "/usr/bin:/bin" };
const tmp = `${(env.TMPDIR ?? "/tmp").replace(/\/+$/, "")}/tinyjs-process-${Date.now()}`;

const drain = async (fd) => {
  const parts = [];
  for (;;) {
    const chunk = await fs.read(fd, 65536);
    if (chunk === null) break;
    parts.push(dec(chunk));
  }
  fs.close(fd);
  return parts.join("");
};

test("process: spawn, pipes, exit code, env isolation", async () => {
  const child = await proc.spawn({ command: "sh", args: ["-c", "echo out-$FOO; echo err >&2; exit 3"], cwd: "/", env: { ...baseEnv, FOO: "bar" }, stdin: "null" });
  assert(child.pid > 0, "pid");
  assertEq(child.stdin, null);
  const [out, err] = await Promise.all([drain(child.stdout), drain(child.stderr)]);
  assertEq(out, "out-bar\n");
  assertEq(err, "err\n");
  assertEq(await proc.wait(child.pid), { code: 3 });
  const printenv = await proc.spawn({ command: "sh", args: ["-c", "echo ${HOME-unset}"], env: baseEnv, stdin: "null" });
  assertEq(await drain(printenv.stdout), "unset\n");
  fs.close(printenv.stderr);
  await proc.wait(printenv.pid);
});

test("process: stdin pipe and cwd", async () => {
  const child = await proc.spawn({ command: "sh", args: ["-c", "cat; pwd"], cwd: "/", env: baseEnv, stdin: "pipe" });
  await fs.write(child.stdin, enc("hello "));
  await fs.write(child.stdin, enc("stdin\n"));
  fs.close(child.stdin);
  assertEq(await drain(child.stdout), "hello stdin\n/\n");
  fs.close(child.stderr);
  assertEq((await proc.wait(child.pid)).code, 0);
});

test("process: spawn errors map to errno", async () => {
  const e = await assertCode(() => proc.spawn({ command: "/nonexistent/prog", env: baseEnv }), "ENOENT", "spawn");
  assertEq(e.path, "/nonexistent/prog");
  await assertCode(() => proc.spawn({ command: "/etc", env: baseEnv }), "EACCES", "spawn");
  await assertCode(() => proc.spawn({ command: "sh", args: ["-c", "true"], cwd: "/nonexistent-cwd", env: baseEnv }), "ENOENT", "spawn");
  await assertCode(() => proc.wait(999999), "ECHILD", "wait");
  await assertCode(() => proc.kill(999999, "SIGTERM"), "ESRCH", "kill");
});

test("process: kill reports the signal", async () => {
  const child = await proc.spawn({ command: "sleep", args: ["30"], env: baseEnv });
  await sleep(20);
  proc.kill(child.pid, "SIGTERM");
  const status = await proc.wait(child.pid);
  assertEq(status, { code: null, signal: "SIGTERM" });
  fs.close(child.stdout); fs.close(child.stderr);
});

test("process: process group kill reaches grandchildren", async () => {
  const child = await proc.spawn({ command: "sh", args: ["-c", "sleep 30 & sleep 30; wait"], env: baseEnv, processGroup: true });
  await sleep(50);
  proc.kill(child.pid, "SIGKILL", { group: true });
  const st = await proc.wait(child.pid);
  // macOS bash 3.2 sometimes reports exit 0 for a group-killed shell (the
  // same happens under Python's killpg); the grandchildren are the point.
  assert(st.signal === "SIGKILL" || st.code === 0, JSON.stringify(st));
  // The grandchild held the pipe: EOF proves the group kill reached it.
  const t0 = performance.now();
  assertEq(await drain(child.stdout), "");
  assert(performance.now() - t0 < 1000, "stdout closed by group kill");
  fs.close(child.stderr);
});

test("process: tee writes full streams, view is bounded, wait counts bytes", async () => {
  await fs.mkdir(tmp, { recursive: true });
  const child = await proc.spawn({
    command: "sh", args: ["-c", "head -c 3000000 /dev/zero | tr '\\0' 'x'; printf abcdef >&2"], env: baseEnv,
    tee: { stdoutPath: `${tmp}/out`, stderrPath: `${tmp}/err`, viewLimit: 1000 },
  });
  const view = await drain(child.stdout);
  assertEq(view.length, 1000);
  assertEq(await drain(child.stderr), "abcdef");
  const status = await proc.wait(child.pid);
  assertEq(status, { code: 0, stdoutBytes: 3000000, stderrBytes: 6 });
  assertEq((await fs.stat(`${tmp}/out`)).size, 3000000);
  assertEq(dec(await fs.readFile(`${tmp}/err`)), "abcdef");
});

test("process: tee drains even when the view is never read", async () => {
  const child = await proc.spawn({
    command: "sh", args: ["-c", "head -c 2000000 /dev/zero"], env: baseEnv,
    tee: { stdoutPath: `${tmp}/out2`, stderrPath: `${tmp}/err2`, viewLimit: 0 },
  });
  const status = await proc.wait(child.pid);
  assertEq(status.stdoutBytes, 2000000);
  fs.close(child.stdout); fs.close(child.stderr);
  await fs.unlink(`${tmp}/out`); await fs.unlink(`${tmp}/err`); await fs.unlink(`${tmp}/out2`); await fs.unlink(`${tmp}/err2`);
  await fs.rmdir(tmp);
});

test("process: close cancels a pending pipe read", async () => {
  const child = await proc.spawn({ command: "sleep", args: ["30"], env: baseEnv });
  const pending = fs.read(child.stdout, 10);
  await sleep(5);
  fs.close(child.stdout);
  await assertCode(() => pending, "ECANCELED", "read");
  proc.kill(child.pid, "SIGKILL");
  await proc.wait(child.pid);
  fs.close(child.stderr);
});

test("process: onSignal receives a signal raised by a child", async () => {
  let got = null;
  onSignal("SIGUSR2", (name) => (got = name));
  const child = await proc.spawn({ command: "kill", args: ["-USR2", String(ownPid)], env: baseEnv });
  await proc.wait(child.pid);
  fs.close(child.stdout); fs.close(child.stderr);
  for (let i = 0; i < 50 && got === null; i++) await sleep(2);
  assertEq(got, "SIGUSR2");
});

test("process: tee throughput (100 MB from /dev/zero, reported as MB/s)", async () => {
  await fs.mkdir(tmp, { recursive: true });
  const t0 = performance.now();
  const child = await proc.spawn({
    command: "head", args: ["-c", "100000000", "/dev/zero"], env: baseEnv,
    tee: { stdoutPath: `${tmp}/tee.out`, stderrPath: `${tmp}/tee.err`, viewLimit: 4096 },
  });
  const view = await drain(child.stdout);
  fs.close(child.stderr);
  const status = await proc.wait(child.pid);
  const seconds = (performance.now() - t0) / 1000;
  assertEq(view.length, 4096);
  assertEq(status.stdoutBytes, 100000000);
  assertEq((await fs.stat(`${tmp}/tee.out`)).size, 100000000);
  console.log(`     tee: ${(100 / seconds).toFixed(0)} MB/s`);
  await fs.unlink(`${tmp}/tee.out`); await fs.unlink(`${tmp}/tee.err`); await fs.rmdir(tmp);
});

test("process: uid/gid switch (root only)", async () => {
  if (ownUid !== 0) { console.log("     (not root: skipped)"); return; }
  const child = await proc.spawn({ command: "sh", args: ["-c", "id -u; id -g"], env: baseEnv, uid: 65534, gid: 65534 });
  assertEq(await drain(child.stdout), "65534\n65534\n");
  fs.close(child.stderr);
  assertEq((await proc.wait(child.pid)).code, 0);
});

test("process: many sequential spawns reap cleanly", async () => {
  for (let i = 0; i < 40; i++) {
    const c = await proc.spawn({ command: "true", env: baseEnv });
    fs.close(c.stdout); fs.close(c.stderr);
    assertEq((await proc.wait(c.pid)).code, 0);
  }
});
