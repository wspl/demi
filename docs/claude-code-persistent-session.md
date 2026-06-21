# Claude Code Persistent Session — Root Cause, Design, Acceptance

This records why the Claude Code provider hit `API Error: 400 ... tool use concurrency`
on multi-turn tool use, why it restarted the `claude` CLI on every turn, and how a single
persistent streaming session fixes both. It follows the same evidence discipline as
`docs/repl-long-run-acceptance.md`: machine-checkable signals over model self-report.

## Symptom

In the web app (and any multi-turn surface), a turn that used a tool succeeded, but the
**next** turn failed with `API Error: 400 ... tool use concurrency issues`. The error never
appears in demi's own source — it comes from the Anthropic API via the `claude` CLI.

## Root cause

The provider drove the CLI **one-shot per turn**: each `streamProviderOnce()` spawned a fresh
`claude` process (`--no-session-persistence`) and replayed the entire transcript into it via
`requestToInputMessages`. Within a turn the process was reused for tool-call continuations, but
it was **killed on the `result` message**, so every new user turn cold-started and replayed.

The replay rebuilt a prior MCP tool call as a **structured** assistant `tool_use` block
(`name: mcp__main__shell_exec`, a synthetic `mcp-control-…` id) plus a `user[tool_result]`,
fed into a process whose SDK-MCP server had just been re-initialized. That specific
combination is what the API rejects as "tool use concurrency".

Captured from the durable wire log (`packages/provider-claude-code/src/wire-log.ts`,
default-on under `$TMPDIR/demi-claude-wire`), the failing turn's input stream was:

```
IN  user      [text]                          (turn 1 prompt)
IN  assistant [thinking, tool_use #mcp-co…]    (replayed turn-1 tool call — structured)
IN  user      [tool_result -> mcp-co…]         (replayed turn-1 tool result)
IN  assistant [text]                           (replayed turn-1 answer)
IN  user      [text]                           (turn 2 prompt)
OUT result    is_error=true :: API Error: 400 ... tool use concurrency
```

Two empirical probes isolated the trigger:

1. Replaying the same structured `assistant[tool_use]` + `user[tool_result]` into a process
   with **no** SDK-MCP server did **not** error. The 400 needs the freshly-initialized MCP
   server **and** the replayed MCP `tool_use` together.
2. One `claude --print --input-format stream-json --no-session-persistence` process **stays
   alive across turns** and **retains conversation context natively** (turn 2 recalled a secret
   word told in turn 1, same `session_id`). So replay is unnecessary — the CLI already keeps the
   full conversation, including structured tool history, in its own context.

## Fix — one persistent streaming session per agent session

Drive the CLI in streaming-input mode and keep the process alive across turns. The provider
instance is created per session (`AgentServer.open`), so each conversation owns one process.

- **Keep alive after `result`** instead of killing. The next turn reuses the process and sends
  only the newly appended user message(s). No restart, no replay.
- **Continuation cursor.** Tool results are matched by `toolUseId` (position-independent, via the
  existing pending-control mechanism). New user turns are detected by an append-only
  `user_message` count, gated on "no pending tool call" so a mid-turn tool continuation and a
  fresh turn boundary stay distinct.
- **Cold start renders tools as text.** When a fresh process must be primed — first turn, resume
  after the process died, or a compaction-driven restart — `coldStartInputMessages` renders prior
  tool calls/results as plain **text**, never structured `tool_use`. That structured replay is the
  sole trigger of the 400; the live path never replays at all.
- **Divergence detection.** A drop in user-message count or a changed leading user message
  (compaction rewrote the transcript) forces a clean cold restart from the rewritten history.
- **Lifecycle.** `AgentProvider.dispose?()` kills the live process; `AgentSession.dispose()` calls
  it, and `AgentServer.closeSession` calls that — so the subprocess is released when the
  connection closes. A per-turn abort listener is removed on normal return so a kept-alive
  process is never killed by a stale turn's signal.

## Acceptance evidence

| Level | Harness | Result |
|---|---|---|
| Unit | `provider.test.ts` regression test | two turns over one transport, one SDK-MCP init, turn 2 writes only the new user message, zero structured `tool_use` replays |
| Real CLI | `AgentSession` + real `claude` driver, 3 tool turns | spawns=1, results=3, "tool use concurrency"=0 |
| Web full stack | browser → WS → backend → real `claude`, 3 tool turns (Claude Opus 4.8) | spawns=1, user-messages-written=3, tool/call-requests=3, concurrency-errors=0 |

Machine-checkable signal in every case: the wire log shows **one** `spawn` for the whole session
and **one** user message written per turn (no replayed history, no structured `tool_use`).

Full suite: `bun run test` → 349 pass / 0 fail; `bun run typecheck` clean.

## Known edges / follow-ups

- **demi compaction vs CLI context.** The live CLI keeps the full conversation in its own context;
  demi's compaction reduces demi's transcript but not the CLI's. In practice compaction drops the
  user-message count, which trips divergence detection and forces a cold restart from the compacted
  (smaller) transcript — so the CLI's context does shrink. A compaction that removes only
  assistant/tool blocks (no user-count change) would leave the CLI's context full; acceptable for
  now (larger context, not a correctness bug). Revisit if long sessions overflow the CLI.
- **Cold-start fidelity.** The text rendering of prior tool calls on resume/compaction is lossy
  (no structured pairing). A future option is the CLI's `--resume <session-id>` to restore native
  structured history instead of replaying text.
