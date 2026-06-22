# Compaction fixture harness

Context-pressure scenarios (compaction, model/provider switch under a smaller window) are awkward
to test because they need a genuinely large conversation. Regenerating hundreds of thousands of
real tokens every run is slow and expensive. So: build one real large-context session **once**,
cache its transcript, and replay it for cheap, repeatable testing.

These call the **real** Claude Code provider (need `claude` auth, cost a little) and are dev tools,
not part of `bun test`.

## What "large context" actually means here

A single model request can never exceed the model's context window (~200k for Sonnet/Opus). So a
"500k-token context" is **not** one giant request — it's a session that has already **compacted
several times**: the cumulative transcript grows past the window while every individual request
stays under it. `Transcript.insertCompactionBoundary` keeps the old blocks (it splices a boundary
in, deleting nothing) and `replayableBlocks()` slices from the last boundary — so the saved
transcript can be 500k+ even though each replayed request is window-bounded.

The fixture is built to be exactly that: a long session that compacted **several generations**
while reading real source files, with three secrets planted at the very start.

## Build the fixture (once)

```sh
bun run scripts/compaction-fixture/build-fixture.ts [targetGenerations]   # default 4
```

Plants three secret codes, then reads/explains large source files under default compaction until
the session has compacted a target number of **generations** (default 4). Targeting generations
(not a token count) is robust regardless of how the size proxy maps to the compactor's real token
estimate — it guarantees a genuine multi-generation session. The secrets get carried through every
compaction summary. Writes the full transcript, gzipped, to `large-context-fixture.json.gz` next to
this README (the raw transcript is multiple MB; gzip keeps the committed blob small). A long build
costs a few dollars.

A pre-built fixture is **committed** alongside these scripts, so the verifier runs immediately
without rebuilding. Re-run the builder to refresh it.

## Verify against the cached fixture (repeatable)

```sh
bun run scripts/compaction-fixture/verify-compaction.ts
```

Loads the fixture via `AgentSession.fromSnapshot` at the **real default compaction thresholds**
(no faking), then asks the model to recall the three secrets planted before the first compaction.
Passes when:

- the loaded session genuinely has **≥ 2 compaction generations** (it's a real long session),
- the model recalls **all three** secrets — i.e. they survived being re-summarized every
  generation,
- no error blocks.

This is the strongest fidelity test: a fact planted at the start must survive several rounds of
summary-of-summary compaction and still be recalled at the end.

This harness is what surfaced two real compaction bugs (summary hijack by instructions buried in
the history → empty summaries; and an unbounded auto-compaction storm), both fixed in
`packages/agent/src/session.ts` and locked by tests in `compaction.test.ts`.

To exercise other scenarios (e.g. switching to a smaller-window model), load the same fixture and
call `session.updateModel(...)` before sending.
