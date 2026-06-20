# Auto Compact Marathon Acceptance Record

| | |
|---|---|
| Date | 2026-06-19 |
| Status | Passed |
| Scope | Real REPL + real Claude Code provider + long multi-turn context pressure |
| Primary model | `claude-haiku-4-5`, thinking off |
| REPL command | `bun run packages/repl/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 1.00 --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Trigger provider-usage auto compact at least 3 times in one REPL session and continue after each compact |

## Scenario Design

Use sequential user turns whose individual text is below the provider-visible truncation threshold, so real provider usage accumulates while send-time preflight does not trigger first. The prompt should ask for minimal acknowledgements and no tool use.

The original completed record for this path is also preserved in `docs/repl-compact-haiku-acceptance.md`, section `Passing real REPL auto compact acceptance`.

## Machine-Checkable Evidence

- `usage>` line appears before each `state> compacting`.
- Trigger usage total is above `0.8 * contextWindow`.
- `state> compacting -> state> running -> state> idle` appears after each trigger.
- Resume usage drops substantially after compact.
- Final process exits cleanly or returns to idle before `/exit`.

## Pass Criteria

- At least 3 auto compacts in one session.
- Each compact is caused by provider usage, not send-time preflight.
- The session continues after every compact.
- No `context_length_exceeded`, no stuck `running`, no provider/tool protocol error.

## Failure Signals

- `cache_write` or `cache_read` exceeds threshold but no compact occurs.
- Compact happens before any triggering `usage>` line.
- Resume usage stays near the pre-compact usage.
- REPL remains `running` after compact without further output.

## Process Record

### Run 1

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-auto-compact-3x-CV5E5q`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-auto-compact-3x-CV5E5q/repl-auto-compact-3x-haiku.log`
- Prompt shape: 37 sequential user turns, each containing about 15k high-entropy ASCII fixture characters and asking for a minimal acknowledgement. Each turn stayed below the 16k provider-visible text truncation threshold.
- Result: exit code 0; REPL returned to idle after the third compact and resume.
- Trigger usages: `172946`, `173235`, `172910`.
- Resume usages: `16186`, `16216`, `16213`.
- Compact phases: 3.
- Final counters: `sent=37`, `compacting=3`, `usage=40`, `idle=38`, `running=40`.
- Verdict: Passed.

| Compact | Sent turns at trigger | Usage event | Trigger usage total | Resume usage total | Outcome |
|---|---:|---:|---:|---:|---|
| 1 | 13 | 13 | `172946` | `16186` | `usage>` exceeded threshold, then `state> compacting -> state> running -> state> idle`. |
| 2 | 25 | 26 | `173235` | `16216` | `usage>` exceeded threshold, then `state> compacting -> state> running -> state> idle`. |
| 3 | 37 | 39 | `172910` | `16213` | `usage>` exceeded threshold, then `state> compacting -> state> running -> state> idle`. |

## Failure Analysis

No acceptance failure remained in the passing run.

Earlier shaping attempts were useful but not sufficient:

- One very large CJK or ASCII user turn did not trigger auto compact because provider-visible text bounding and provider usage kept total pressure below threshold.
- A single huge user block is the wrong shape for this acceptance because `Transcript.collectInferenceItems()` bounds model-visible text at 8k head + 8k tail.
- Multi-turn sub-16k chunks are the accepted shape because they accumulate real provider context pressure without preflight compact firing first.

## Follow-Up Deterministic Tests

The passing run led to deterministic coverage around `AgentSession.isUsageNearLimit()` and cache usage semantics:

- `auto compaction counts cache usage as context pressure`
- `cache usage is recorded without leaking into model context or breaking tool loop`

Future failures should add or update tests around provider usage mapping, compaction continuation request shape, and cache-backed context pressure.
