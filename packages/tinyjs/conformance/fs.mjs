import { test, assert, assertEq, assertBytes, assertCode } from "./harness.mjs";
import * as fs from "tinyjs:fs";
import { env } from "tinyjs:runtime";

const enc = (s) => new TextEncoder().encode(s);
const dec = (b) => new TextDecoder().decode(b);
const tmp = `${env.TMPDIR ?? "/tmp"}/tinyjs-conformance-${Date.now()}-${Math.floor(Math.random() * 1e6)}`;
const p = (name) => `${tmp}/${name}`;

test("fs: mkdir/readdir/stat/rename/unlink/rmdir", async () => {
  await fs.mkdir(p("a/b/c"), { recursive: true });
  await fs.writeFile(p("a/b/c/f.txt"), enc("hello"));
  await fs.writeFile(p("a/b/c/f.txt"), enc(" world"), { append: true });
  assertEq(dec(await fs.readFile(p("a/b/c/f.txt"))), "hello world");
  const st = await fs.stat(p("a/b/c/f.txt"));
  assertEq(st.kind, "file");
  assertEq(st.size, 11);
  assert(typeof st.mtimeMs === "number" && st.mtimeMs > 1e12, "mtimeMs");
  assert(typeof st.ino === "number" && typeof st.uid === "number", "ino/uid");
  assertEq((await fs.stat(p("a"))).kind, "dir");
  const entries = await fs.readdir(p("a/b/c"));
  assertEq(entries, [{ name: "f.txt", kind: "file" }]);
  await fs.rename(p("a/b/c/f.txt"), p("g.txt"));
  await fs.unlink(p("g.txt"));
  await fs.rmdir(p("a/b/c"));
  assertEq((await fs.readdir(p("a/b"))), []);
});

test("fs: errno fidelity", async () => {
  const e = await assertCode(() => fs.readFile(p("missing")), "ENOENT", "open");
  assertEq(e.path, p("missing"));
  assert(/ENOENT: no such file or directory, open '/.test(e.message), e.message);
  await assertCode(() => fs.mkdir(p("a")), "EEXIST", "mkdir");
  await assertCode(() => fs.rmdir(p("a")), "ENOTEMPTY", "rmdir");
  await assertCode(() => fs.readFile(p("a")), "EISDIR", "open");
  await assertCode(() => fs.readdir(p("a/b/nope")), "ENOENT", "scandir");
  await assertCode(() => fs.stat(`${p("a")}/x/y`), "ENOENT", "stat");
  await fs.writeFile(p("file"), enc("x"));
  await assertCode(() => fs.mkdir(p("file/sub")), "ENOTDIR", "mkdir");
  await assertCode(() => fs.read(9999, 10), "EBADF", "read");
  await assertCode(() => fs.close(9999), "EBADF", "close");
  if (env.USER !== "root") {
    await fs.chmod(p("file"), 0o000);
    await assertCode(() => fs.readFile(p("file")), "EACCES", "open");
    await fs.chmod(p("file"), 0o644);
  }
});

test("fs: symlink/link/readlink/realpath/chmod/utimes/truncate", async () => {
  await fs.writeFile(p("target"), enc("12345"));
  await fs.symlink(p("target"), p("sym"));
  assertEq(await fs.readlink(p("sym")), p("target"));
  assertEq((await fs.lstat(p("sym"))).kind, "symlink");
  assertEq((await fs.stat(p("sym"))).kind, "file");
  assert((await fs.realpath(p("sym"))).endsWith("/target"), "realpath follows");
  await fs.link(p("target"), p("hard"));
  assertEq((await fs.stat(p("hard"))).nlink, 2);
  await fs.chmod(p("target"), 0o600);
  assertEq((await fs.stat(p("target"))).mode & 0o777, 0o600);
  await fs.utimes(p("target"), 1_000_000_000_000, 1_500_000_000_500);
  assertEq(Math.round((await fs.stat(p("target"))).mtimeMs), 1_500_000_000_500);
  await fs.truncate(p("target"), 2);
  assertEq(dec(await fs.readFile(p("target"))), "12");
  const entries = (await fs.readdir(tmp)).filter((e) => e.name === "sym");
  assertEq(entries, [{ name: "sym", kind: "symlink" }]);
});

test("fs: streaming open/read/write/close", async () => {
  const fd = await fs.open(p("stream"), "w", 0o640);
  await fs.write(fd, enc("abc"));
  await fs.write(fd, enc("def"));
  fs.close(fd);
  assertEq((await fs.stat(p("stream"))).mode & 0o777, 0o640);
  const rd = await fs.open(p("stream"), "r");
  assertEq(dec(await fs.read(rd, 4)), "abcd");
  assertEq(dec(await fs.read(rd, 4)), "ef");
  assertEq(await fs.read(rd, 4), null);
  // pread: a position leaves the cursor alone; a stream has none.
  assertEq(dec(await fs.read(rd, 3, 2)), "cde");
  assertEq(await fs.read(rd, 4), null);
  assertEq(await fs.read(rd, 10, 6), null);
  await assertCode(() => fs.read(rd, 4, -1), "EINVAL", "read");
  fs.close(rd);
  await assertCode(() => fs.open(p("stream"), "wx"), "EEXIST", "open");
  await assertCode(() => fs.read(rd, 1), "EBADF", "read");
  const ap = await fs.open(p("stream"), "a");
  await fs.write(ap, enc("!"));
  fs.close(ap);
  assertEq(dec(await fs.readFile(p("stream"))), "abcdef!");
});

test("fs: large file in one allocation and chunked reads", async () => {
  const size = 8 * 1024 * 1024;
  const data = new Uint8Array(size);
  for (let i = 0; i < size; i += 4096) data[i] = i & 0xff;
  await fs.writeFile(p("big"), data);
  const back = await fs.readFile(p("big"));
  assertEq(back.length, size);
  assertEq(back[4096 * 3], (4096 * 3) & 0xff);
  const fd = await fs.open(p("big"), "r");
  let total = 0, chunks = 0;
  for (;;) {
    const chunk = await fs.read(fd, 1024 * 1024);
    if (chunk === null) break;
    total += chunk.length;
    chunks++;
  }
  fs.close(fd);
  assertEq(total, size);
  assertEq(chunks, 8);
});

test("fs: pipe carries bytes in order with backpressure; close ends or breaks the other side", async () => {
  const { read, write } = fs.pipe();
  const total = 4 * 1024 * 1024;
  const chunk = new Uint8Array(64 * 1024);
  for (let i = 0; i < chunk.length; i++) chunk[i] = i & 0xff;
  // The writer runs ahead only as far as the buffer allows: a 4 MB write completes only as the reader drains.
  let written = 0;
  const writer = (async () => {
    for (let at = 0; at < total; at += chunk.length) {
      await fs.write(write, chunk);
      written += chunk.length;
    }
    fs.close(write);
  })();
  await new Promise((r) => setTimeout(r, 50));
  assert(written < total, `the writer blocked at ${written} bytes`);
  let read_total = 0, mismatch = false;
  for (;;) {
    const got = await fs.read(read, 100000);
    if (got === null) break;
    for (let i = 0; i < got.length; i++) if (got[i] !== ((read_total + i) % chunk.length & 0xff)) mismatch = true;
    read_total += got.length;
  }
  await writer;
  assertEq(read_total, total);
  assert(!mismatch, "bytes arrived in order");
  fs.close(read);
  // The reader gone: the writer sees EPIPE (after the buffer, at once here since the buffer is empty).
  const other = fs.pipe();
  fs.close(other.read);
  await assertCode(() => fs.write(other.write, new Uint8Array(1024 * 1024)), "EPIPE", "write");
  fs.close(other.write);
});

test("fs: cleanup", async () => {
  const rm = async (path) => {
    const st = await fs.lstat(path);
    if (st.kind === "dir") {
      for (const e of await fs.readdir(path)) await rm(`${path}/${e.name}`);
      await fs.rmdir(path);
    } else {
      await fs.unlink(path);
    }
  };
  await rm(tmp);
  await assertCode(() => fs.stat(tmp), "ENOENT");
});
