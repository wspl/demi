# Compaction fixture harness

Context-pressure scenarios (compaction, model/provider switch under a smaller window) are awkward
to test because they need a genuinely large conversation. Regenerating tens of thousands of real
tokens every run is slow and expensive. So: build one real large-context conversation **once**,
cache its transcript, and replay it for cheap, repeatable testing.

These call the **real** Claude Code provider (need `claude` auth, cost a little) and are dev tools,
not part of `bun test`.

## Build the fixture (once)

```sh
bun run scripts/compaction-fixture/build-fixture.ts
```

Runs a real conversation that plants three secret codes and then reads/explains several source
files until the context is large, then writes the transcript to
`.test-cache/large-context-fixture.json` (gitignored).

## Verify compaction against the cached fixture (repeatable)

```sh
bun run scripts/compaction-fixture/verify-compaction.ts
```

Loads the fixture via `AgentSession.fromSnapshot`, sets a realistic threshold so the loaded context
triggers exactly one compaction, then asks the model to recall the planted secrets. Passes when:

- exactly one (≤ a few) compaction boundary is created — no storm,
- the boundary summary is real and non-empty,
- the model recalls all three secrets from the summary,
- no error blocks.

This harness is what surfaced two real compaction bugs (summary hijack by instructions buried in
the history → empty summaries; and an unbounded auto-compaction storm), both fixed in
`packages/agent/src/session.ts` and locked by tests in `compaction.test.ts`.

To exercise other scenarios (e.g. switching to a smaller-window model), load the same fixture and
call `session.updateModel(...)` before sending.
