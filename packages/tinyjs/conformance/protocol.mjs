import { test, assert, assertEq } from "./harness.mjs";
import { env, identity } from "tinyjs:runtime";
import { msgpackEncode, msgpackDecode } from "tinyjs:bytes";

// The runner-protocol wire, bundled by the integration test with Bun, must
// encode and decode on tinyjs over the tinyjs:bytes codec: the bundle is a
// file-loaded module and sees only the standard globals, so the codec is
// handed in.
const bundle = env.TINYJS_CONFORMANCE_PROTOCOL;

test(bundle ? "protocol: runner-protocol bundle frames on tinyjs over tinyjs:bytes" : "protocol: bundle case skipped (set TINYJS_CONFORMANCE_PROTOCOL)", async () => {
  if (!bundle) return;
  const rp = await import(bundle);
  const wire = rp.createRunnerWire({ encode: msgpackEncode, decode: msgpackDecode });
  const hello = {
    type: "hello",
    protocol: rp.RUNNER_PROTOCOL_VERSION,
    runner: { name: "tinyjs", platform: "tinyjs", version: "0.0.1", identity: { uid: identity.uid, gid: identity.gid, hostname: identity.hostname, homeDir: identity.homeDir } },
  };
  const frame = wire.encode(hello);
  assert(frame instanceof Uint8Array, "frame is bytes");
  const back = wire.decodeRunnerToBackend(frame);
  assertEq(back.type, "hello");
  assertEq(back.runner.name, "tinyjs");
  const call = wire.decodeBackendToRunner(wire.encode({ type: "fs_utimes", id: "c1", path: "/x", atime: new Date(1_600_000_000_000), mtime: new Date(1_600_000_000_250) }));
  assert(call.atime instanceof Date, "Date survives");
  assertEq(call.mtime.getTime(), 1_600_000_000_250);
  const ok = wire.decodeRunnerToBackend(wire.encode({ type: "fs_ok", id: "c1", op: "readFile", result: new Uint8Array([1, 2, 3]) }));
  assertEq(Array.from(ok.result), [1, 2, 3]);
  let threw = false;
  try { wire.decodeBackendToRunner(wire.encode({ type: "spawn", spawnId: 5 })); } catch { threw = true; }
  assert(threw, "schema rejects a bad frame");
});
