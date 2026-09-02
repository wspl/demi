import { test, assert, assertEq, assertBytes, assertCode, assertThrows, sleep } from "./harness.mjs";
import * as net from "tinyjs:net";
import * as fs from "tinyjs:fs";
import * as proc from "tinyjs:process";
import { env, openHandles } from "tinyjs:runtime";

const enc = (s) => new TextEncoder().encode(s);
const dec = (b) => new TextDecoder().decode(b);
const tmp = `${(env.TMPDIR ?? "/tmp").replace(/\/+$/, "")}/tinyjs-net-${Date.now()}`;

const drain = async (fd) => {
  const parts = [];
  let total = 0;
  for (;;) {
    const chunk = await fs.read(fd, 1024 * 1024);
    if (chunk === null) break;
    parts.push(chunk);
    total += chunk.length;
  }
  fs.close(fd);
  const out = new Uint8Array(total);
  let off = 0;
  for (const p of parts) { out.set(p, off); off += p.length; }
  return out;
};

test("net: uds listen/connect/accept with mode, read/write both ways, close cancels accept", async () => {
  await fs.mkdir(tmp, { recursive: true });
  const path = `${tmp}/relay.sock`;
  const listener = await net.udsListen(path, { mode: 0o600 });
  assertEq((await fs.stat(path)).mode & 0o777, 0o600);
  const accepted = net.accept(listener);
  const client = await net.udsConnect(path);
  const server = await accepted;
  await fs.write(client, enc("ping"));
  assertEq(dec(await fs.read(server, 100)), "ping");
  await fs.write(server, enc("pong"));
  assertEq(dec(await fs.read(client, 100)), "pong");
  // Both directions may be in flight at once on a socket.
  const pendingRead = fs.read(client, 100);
  await fs.write(client, enc("x"));
  await fs.write(server, enc("y"));
  assertEq(dec(await pendingRead), "y");
  assertEq(dec(await fs.read(server, 100)), "x");
  fs.close(client);
  assertEq(await fs.read(server, 100), null);
  fs.close(server);
  const pendingAccept = net.accept(listener);
  await sleep(5);
  net.close(listener);
  await assertCode(() => pendingAccept, "ECANCELED", "accept");
  await assertCode(() => net.udsConnect(`${tmp}/missing.sock`), "ENOENT", "connect");
  await assertCode(() => net.udsListen(path, { mode: 0o600 }), "EADDRINUSE", "listen");
  await fs.unlink(path);
});

// The stub-backed cases need Bun and the ports the integration test hands
// out; without them they are skipped.
const stubDir = env.TINYJS_CONFORMANCE_DIR;
const ports = (env.TINYJS_CONFORMANCE_PORTS ?? "").split(",").map(Number);
const bun = env.TINYJS_CONFORMANCE_BUN;
if (stubDir && bun && ports.length === 3) {
  const [httpPort, tlsPort, proxyPort] = ports;
  const http = `http://127.0.0.1:${httpPort}`;
  const https = `https://localhost:${tlsPort}`;
  const tls = !!env.TINYJS_CONFORMANCE_CERT;
  let stub;

  test("net: stub starts", async () => {
    stub = await proc.spawn({
      command: bun, args: [`${stubDir}/stub.ts`], cwd: stubDir,
      env: { PATH: env.PATH, HOME: env.HOME ?? "/tmp", TINYJS_CONFORMANCE_PORTS: ports.join(","), TINYJS_CONFORMANCE_CERT: env.TINYJS_CONFORMANCE_CERT ?? "", TINYJS_CONFORMANCE_KEY: env.TINYJS_CONFORMANCE_KEY ?? "" },
    });
    const line = dec(await fs.read(stub.stdout, 100));
    assert(line.startsWith("ready"), `stub said: ${line}`);
  });

  test("net: http GET with headers and a streamed 16 MB body", async () => {
    const r = await net.httpRequest({ method: "GET", url: `${http}/hello`, headers: { "x-test": "1" } });
    assertEq(r.status, 200);
    assertEq(r.headers["x-stub"], "yes");
    assertEq(dec(await drain(r.body)), "hello");
    const big = await net.httpRequest({ method: "GET", url: `${http}/big`, headers: {} });
    let total = 0, chunks = 0;
    for (;;) {
      const c = await fs.read(big.body, 1024 * 1024);
      if (c === null) break;
      total += c.length; chunks++;
    }
    fs.close(big.body);
    assertEq(total, 16 * 1024 * 1024);
    assert(chunks >= 16, `streamed in ${chunks} chunks`);
    const chunked = await net.httpRequest({ method: "GET", url: `${http}/chunked`, headers: {} });
    assertEq(dec(await drain(chunked.body)), "chunk0;chunk1;chunk2;chunk3;chunk4;");
    const status = await net.httpRequest({ method: "GET", url: `${http}/status?code=503`, headers: {} });
    assertEq(status.status, 503);
    fs.close(status.body);
  });

  test("net: http request bodies from bytes and from a file", async () => {
    const bytes = new Uint8Array(100000);
    for (let i = 0; i < bytes.length; i++) bytes[i] = i & 0xff;
    let sum = 0;
    for (const b of bytes) sum = (sum + b) & 0xffff;
    const r1 = await net.httpRequest({ method: "PUT", url: `${http}/upload`, headers: { "content-type": "application/octet-stream" }, body: bytes });
    assertEq(dec(await drain(r1.body)), `got 100000 sum ${sum}`);
    const path = `${tmp}/upload.bin`;
    const file = new Uint8Array(5 * 1024 * 1024);
    for (let i = 0; i < file.length; i += 997) file[i] = 1;
    await fs.writeFile(path, file);
    const r2 = await net.httpRequest({ method: "PUT", url: `${http}/upload`, headers: {}, body: { file: path } });
    assertEq(dec(await drain(r2.body)), `got ${file.length} sum ${Math.ceil(file.length / 997) & 0xffff}`);
    await fs.unlink(path);
    await assertCode(() => net.httpRequest({ method: "PUT", url: `${http}/upload`, headers: {}, body: { file: `${tmp}/nope` } }), "ENOENT", "open");
    const h = await net.httpRequest({ method: "GET", url: `${http}/headers`, headers: { "x-a": "1", "X-B": "two" } });
    const seen = JSON.parse(dec(await drain(h.body)));
    assertEq([seen["x-a"], seen["x-b"], seen["host"]], ["1", "two", `127.0.0.1:${httpPort}`]);
  });

  test("net: http connection errors carry errno codes", async () => {
    await assertCode(() => net.httpRequest({ method: "GET", url: "http://127.0.0.1:1/x", headers: {} }), "ECONNREFUSED", "connect");
    await assertCode(() => net.httpRequest({ method: "GET", url: "ftp://x/", headers: {} }), "EINVAL", "request");
  });

  test("net: websocket echo, text frames, server close yields null", async () => {
    const ws = await net.wsConnect(`ws://127.0.0.1:${httpPort}/ws`, { headers: { "x-hello": "ws" } });
    const payload = new Uint8Array(300000);
    for (let i = 0; i < payload.length; i++) payload[i] = (i * 7) & 0xff;
    await net.wsSend(ws, payload);
    const back = await net.wsRecv(ws);
    assertBytes(back, payload);
    await net.wsClose(ws, 1000);
    await assertCode(() => net.wsSend(ws, payload), "EBADF", "send");
    const closing = await net.wsConnect(`ws://127.0.0.1:${httpPort}/ws-close-now`);
    assertEq(await net.wsRecv(closing), null);
    await net.wsClose(closing);
  });

  test("net: websocket receive keeps up with a 64 MB flood; close cancels a pending recv", async () => {
    const ws = await net.wsConnect(`ws://127.0.0.1:${httpPort}/ws-flood`);
    let frames = 0, bytes = 0;
    for (;;) {
      const f = await net.wsRecv(ws);
      if (f === null) break;
      frames++; bytes += f.length;
    }
    assertEq([frames, bytes], [64, 64 * 1024 * 1024]);
    await net.wsClose(ws);
    const idle = await net.wsConnect(`ws://127.0.0.1:${httpPort}/ws`);
    const pending = net.wsRecv(idle);
    await sleep(5);
    net.close(idle);
    await assertCode(() => pending, "ECANCELED", "recv");
  });

  test("net: websocket send backpressure: 32 MB in 1 MB frames all echoed", async () => {
    const ws = await net.wsConnect(`ws://127.0.0.1:${httpPort}/ws`);
    const frame = new Uint8Array(1024 * 1024).fill(9);
    let received = 0;
    const reader = (async () => {
      while (received < 32 * 1024 * 1024) received += (await net.wsRecv(ws)).length;
    })();
    for (let i = 0; i < 32; i++) await net.wsSend(ws, frame);
    await reader;
    assertEq(received, 32 * 1024 * 1024);
    await net.wsClose(ws);
  });

  test("net: https and wss through the CONNECT proxy with the test CA", async () => {
    if (!tls) { console.log("     (no certificate: skipped)"); return; }
    assert(env.HTTPS_PROXY === `http://user:pass@127.0.0.1:${proxyPort}`, "HTTPS_PROXY set by the runner");
    const r = await net.httpRequest({ method: "GET", url: `${https}/hello`, headers: {} });
    assertEq(r.status, 200);
    assertEq(dec(await drain(r.body)), "hello");
    const ws = await net.wsConnect(`wss://localhost:${tlsPort}/ws`);
    await net.wsSend(ws, enc("secure"));
    assertEq(dec(await net.wsRecv(ws)), "secure");
    await net.wsClose(ws);
    // A TLS handshake against the plain HTTP port fails inside connect.
    await assertCode(() => net.httpRequest({ method: "GET", url: `https://localhost:${httpPort}/hello`, headers: {} }), "EPROTO", "connect");
  });

  test("net: stub stops and no handles leak", async () => {
    proc.kill(stub.pid, "SIGKILL");
    await proc.wait(stub.pid);
    fs.close(stub.stdout); fs.close(stub.stderr);
    await fs.rmdir(tmp);
    assertEq(openHandles(), 0);
  });
} else {
  test("net: stub-backed cases skipped (set TINYJS_CONFORMANCE_DIR/PORTS/BUN)", async () => {
    await fs.rmdir(tmp);
  });
}
