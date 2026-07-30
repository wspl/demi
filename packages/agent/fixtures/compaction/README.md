# Compaction fixture harness

Context-pressure scenarios (compaction, model/provider switch under a smaller window)
are awkward to unit-test: they need a genuinely large conversation. Regenerating
hundreds of thousands of real tokens every run is slow and expensive. So: build one
real large-context session **once**, cache its transcript, and replay it for cheap,
repeatable verification.

These call the **real** DeepSeek V4 Flash API (`DEEPSEEK_API_KEY` in the repo-root
`.env`) and are **not** part of `bun test`.

## What "large context" means

A single model request can never exceed the registered context window. A
"500k-token context" is a session that has already **compacted several times**: the
cumulative transcript grows past the window while every individual request stays
under it. The committed fixture is that kind of multi-generation session, with three
secrets planted at the start.

## Commands

```sh
bun run fixtures:compaction:build [targetGenerations]   # optional refresh; default 4
bun run fixtures:compaction:verify                      # recall (+ EXTRA_GENERATIONS forced rounds)
bun run fixtures:compaction:verify-switch               # small↔large window switch on flash
```

Tune depth by editing constants at the top of `verify-compaction.ts`
(`EXTRA_GENERATIONS`, `KEEP_RECENT`, `VERIFY_CONTEXT_WINDOW`).

Optional env knobs:

| Env | Default | Meaning |
|---|---|---|
| `DEEPSEEK_API_KEY` | — | required |
| `DEEPSEEK_BASE_URL` | `https://api.deepseek.com/v1` | API base |
| `DEEPSEEK_FLASH_MODEL` | `deepseek-v4-flash` | model id |
| `COMPACTION_FIXTURE_SMALL_WINDOW` | `8000` | switch harness small window |
| `COMPACTION_FIXTURE_LARGE_WINDOW` | `400000` | switch harness large window |
| `COMPACTION_FIXTURE_GROW_TARGET` | `0` | optional extra grow after compactable-window is met |
| `COMPACTION_FIXTURE_BUILD_WINDOW` | `100000` | registered window while building |

## Verify recall

`verify-compaction.ts` loads the committed fixture via `AgentSession.fromCheckpoint`,
asks flash to recall the three secrets, then forces `EXTRA_GENERATIONS` more
compaction rounds (grow → compact → recall). It prints a generation table and exits
non-zero when recall drops below 3/3.

## Verify window switch

`verify-window-switch.ts` uses two DeepSeek Flash registrations (small vs large
`contextWindow`) on the same fixture:

1. small → large must **not** compact
2. grow on the large window
3. large → small must **force** compaction by the pre-switch model; secrets still recall
4. switch back to large; secrets still recall

## Design link

Compaction summaries run through `AgentSession.clone()` + the normal turn path so
prefix caches can reuse the conversation prefix. See `docs/compaction-context-cache.md`
and `docs/provider-session-clone.md`.
