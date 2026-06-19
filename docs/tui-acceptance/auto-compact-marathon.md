# Auto Compact Marathon Acceptance Record

| | |
|---|---|
| Date | TBD |
| Status | Designed |
| Scope | Real TUI + real Claude Code provider + long multi-turn context pressure |
| Primary model | `claude-haiku-4-5`, thinking off |
| TUI command | `bun run packages/tui/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 1.00 --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Trigger provider-usage auto compact at least 3 times in one TUI session and continue after each compact |

## Scenario Design

Use sequential user turns whose individual text is below the provider-visible truncation threshold, so real provider usage accumulates while send-time preflight does not trigger first. The prompt should ask for minimal acknowledgements and no tool use.

The existing completed record for this path is in `docs/tui-compact-haiku-acceptance.md`, section `Passing real TUI auto compact acceptance`.

## Machine-Checkable Evidence

- `usage:` line appears before each `status: compacting`.
- Trigger usage total is above `0.8 * contextWindow`.
- `status: compacting -> status: running -> status: idle` appears after each trigger.
- Resume usage drops substantially after compact.
- Final process exits cleanly or returns to idle before `/exit`.

## Pass Criteria

- At least 3 auto compacts in one session.
- Each compact is caused by provider usage, not send-time preflight.
- The session continues after every compact.
- No `context_length_exceeded`, no stuck `running`, no provider/tool protocol error.

## Failure Signals

- `cache_write` or `cache_read` exceeds threshold but no compact occurs.
- Compact happens before any triggering `usage:` line.
- Resume usage stays near the pre-compact usage.
- TUI remains `running` after compact without further output.

## Process Record

### Run 1

- Date: TBD
- Workspace: TBD
- Log path: TBD
- Prompt shape: TBD
- Result: TBD
- Trigger usages: TBD
- Compact phases: TBD
- Verdict: TBD

## Follow-Up Deterministic Tests

When this fails, add or update tests around `AgentSession.isUsageNearLimit()`, provider usage mapping, and compaction continuation request shape.
