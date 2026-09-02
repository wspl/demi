import { test, assert, assertEq, assertBytes } from "./harness.mjs";
import { msgpackEncode, msgpackDecode, base64Encode, base64Decode, sha256, randomBytes } from "demishell:bytes";

const hex = (b) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");

test("bytes: base64", () => {
  assertEq(base64Encode(new Uint8Array([104, 105])), "aGk=");
  assertBytes(base64Decode("aGk="), [104, 105]);
  assertBytes(base64Decode("aGk"), [104, 105]);
  assertBytes(base64Decode("_-8"), [255, 239]);
  const big = randomBytes(10000);
  assertBytes(base64Decode(base64Encode(big)), big);
});

test("bytes: sha256 and randomBytes", () => {
  assertEq(hex(sha256(new Uint8Array(0))), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
  assertEq(hex(sha256(new TextEncoder().encode("abc"))), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
  const a = randomBytes(16), b = randomBytes(16);
  assertEq(a.length, 16);
  assert(hex(a) !== hex(b), "random differs");
});

test("bytes: msgpack scalars and containers", () => {
  const value = { n: 1, neg: -5, big: 2 ** 40, f: 1.5, s: "héllo", t: true, z: null, u: undefined, a: [1, "x", [2]], o: { k: "v" } };
  const encoded = msgpackEncode(value);
  const decoded = msgpackDecode(encoded);
  assertEq(decoded, { n: 1, neg: -5, big: 2 ** 40, f: 1.5, s: "héllo", t: true, z: null, u: null, a: [1, "x", [2]], o: { k: "v" } });
  assertEq(hex(msgpackEncode(1)), "01");
  assertEq(hex(msgpackEncode(-1)), "ff");
  assertEq(hex(msgpackEncode(256)), "cd0100");
  assertEq(hex(msgpackEncode("a")), "a161");
  assertEq(hex(msgpackEncode([])), "90");
  assertEq(hex(msgpackEncode({})), "80");
  assertEq(hex(msgpackEncode(1.5)), "cb3ff8000000000000");
});

test("bytes: msgpack extension types", () => {
  const bin = new Uint8Array([0, 255, 7]);
  assertEq(hex(msgpackEncode(bin)), "c40300ff07");
  assertBytes(msgpackDecode(msgpackEncode({ b: bin })).b, bin);
  const d = new Date(1700000000000);
  assertEq(hex(msgpackEncode(d)), "d6ff6553f100");
  const d2 = new Date(1700000000123);
  const back = msgpackDecode(msgpackEncode({ d: d2 })).d;
  assert(back instanceof Date, "Date decoded");
  assertEq(back.getTime(), 1700000000123);
  assertEq(msgpackDecode(msgpackEncode(new Date(-1000))).getTime(), -1000);
});

test("bytes: msgpack errors", () => {
  let threw = false;
  try { msgpackDecode(new Uint8Array([0x92, 0x01])); } catch (e) { threw = e instanceof TypeError && /msgpackDecode/.test(e.message); }
  assert(threw, "truncated input throws");
  threw = false;
  try { msgpackEncode({ f() {} }); } catch (e) { threw = true; }
  assert(threw, "function throws");
});
