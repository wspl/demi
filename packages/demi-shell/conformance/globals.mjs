import { test, assert, assertEq, assertBytes } from "./harness.mjs";

test("globals: text encoding round trip", () => {
  const enc = new TextEncoder();
  const dec = new TextDecoder();
  const text = "héllo 世界 🚀";
  const bytes = enc.encode(text);
  assertEq(bytes.length, 18);
  assertEq(dec.decode(bytes), text);
  assertEq(dec.decode(new Uint8Array([0xef, 0xbb, 0xbf, 0x41])), "A", "BOM stripped");
  assertEq(new TextDecoder("utf-8", { ignoreBOM: true }).decode(new Uint8Array([0xef, 0xbb, 0xbf, 0x41])), "﻿A");
});

test("globals: streaming decode splits sequences", () => {
  const dec = new TextDecoder();
  const bytes = new TextEncoder().encode("a世b");
  const out = dec.decode(bytes.subarray(0, 2), { stream: true }) + dec.decode(bytes.subarray(2, 4), { stream: true }) + dec.decode(bytes.subarray(4));
  assertEq(out, "a世b");
  assertEq(new TextDecoder().decode(new Uint8Array([0x61, 0xff, 0x62])), "a�b");
  let threw = false;
  try { new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array([0xff])); } catch (e) { threw = e instanceof TypeError; }
  assert(threw, "fatal throws TypeError");
});

test("globals: atob/btoa", () => {
  assertEq(btoa("hello"), "aGVsbG8=");
  assertEq(atob("aGVsbG8="), "hello");
  assertEq(atob("aGVsbG8"), "hello");
  assertEq(btoa("\xff\x00"), "/wA=");
});

test("globals: URL", () => {
  const u = new URL("HTTPS://User:pw@Example.com:8443/a/./b/../c?x=1&y=2#frag");
  assertEq(u.protocol, "https:");
  assertEq(u.username, "User");
  assertEq(u.hostname, "example.com");
  assertEq(u.port, "8443");
  assertEq(u.pathname, "/a/c");
  assertEq(u.search, "?x=1&y=2");
  assertEq(u.hash, "#frag");
  assertEq(u.origin, "https://example.com:8443");
  assertEq(u.searchParams.get("y"), "2");
  u.searchParams.set("z", "a b");
  assertEq(u.search, "?x=1&y=2&z=a+b");
  assertEq(new URL("wss://relay.example:443/ws").port, "");
  assertEq(new URL("../x?q", "http://h/a/b/c").href, "http://h/a/x?q");
  assertEq(new URL("/root", "http://h/a/b").href, "http://h/root");
  assertEq(new URL("unix:/run/demi.sock").pathname, "/run/demi.sock");
  assertEq(URL.canParse("nope"), false);
  const p = new URLSearchParams("a=1&b=%20x&a=2");
  assertEq(p.getAll("a"), ["1", "2"]);
  assertEq(p.get("b"), " x");
});

test("globals: AbortController and structuredClone", () => {
  const c = new AbortController();
  let fired = 0;
  c.signal.addEventListener("abort", () => fired++);
  c.abort();
  c.abort();
  assertEq(fired, 1);
  assertEq(c.signal.aborted, true);
  assertEq(c.signal.reason.name, "AbortError");
  const src = { a: [1, { b: new Uint8Array([1, 2]) }], d: new Date(0), m: new Map([["k", 1]]) };
  src.self = src;
  const copy = structuredClone(src);
  assert(copy !== src && copy.self === copy, "cycle preserved");
  assertBytes(copy.a[1].b, [1, 2]);
  assertEq(copy.d.getTime(), 0);
  assertEq(copy.m.get("k"), 1);
});

test("globals: crypto and performance", () => {
  const a = crypto.randomUUID();
  assert(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(a), a);
  const buf = new Uint8Array(32);
  crypto.getRandomValues(buf);
  assert(buf.some((b) => b !== 0), "filled");
  const t = performance.now();
  assert(typeof t === "number" && t >= 0);
});

test("globals: queueMicrotask ordering", async () => {
  const order = [];
  queueMicrotask(() => order.push("micro"));
  setTimeout(() => order.push("macro"), 0);
  order.push("sync");
  await new Promise((r) => setTimeout(r, 5));
  assertEq(order, ["sync", "micro", "macro"]);
});

test("globals: console formats values", () => {
  console.log("plain", 1, { a: [1, "x"], b: null }, new Uint8Array([1]), undefined);
  console.log("%s=%d %j", "n", 42, { ok: true });
});
