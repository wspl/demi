import { test, assert, assertEq } from "./harness.mjs";
import { env, identity } from "tinyjs:runtime";

// The runner-protocol codec, bundled by the integration test with Bun, must
// encode and decode on tinyjs: it is a file-loaded module, so it sees
// only the standard globals.
const bundle = env.TINYJS_CONFORMANCE_PROTOCOL;

test(bundle ? "protocol: runner-protocol bundle encodes and decodes on tinyjs" : "protocol: bundle case skipped (set TINYJS_CONFORMANCE_PROTOCOL)", async () => {
  if (!bundle) return;
  const rp = await import(bundle);
  const hello = {
    type: "hello",
    protocol: rp.RUNNER_PROTOCOL_VERSION,
    runner: { name: "tinyjs", platform: "tinyjs", version: "0.0.1", identity: { ...identity, homeDir: identity.homeDir } },
  };
  const frame = rp.encodeRunnerMessage(hello);
  assert(typeof frame === "string", "frame is a string");
  const back = rp.decodeRunnerToBackendMessage(frame);
  assertEq(back.type, "hello");
  assertEq(back.runner.name, "tinyjs");
  const spawn = rp.decodeBackendToRunnerMessage(rp.encodeRunnerMessage({ type: "spawn", spawnId: "s1", command: "ls", args: ["-l"] }));
  assertEq(spawn.args, ["-l"]);
  let threw = false;
  try { rp.decodeBackendToRunnerMessage(rp.encodeRunnerMessage({ type: "spawn", spawnId: 5 })); } catch { threw = true; }
  assert(threw, "schema rejects a bad frame");
});
