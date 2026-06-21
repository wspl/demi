# Tool-Heavy Compact Marathon Acceptance Record

| | |
|---|---|
| Date | 2026-06-19 |
| Status | Passed |
| Scope | Real REPL + real Claude Code provider + shell tools + repeated compact |
| Primary model | `claude-haiku-4-5`, thinking off |
| REPL command | `bun run packages/repl/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Trigger compact repeatedly while the model must keep using `shell_exec` after compact |

## Scenario Design

Ask for several finite shell-backed benchmark cycles. Each cycle should require a small number of distinct `shell_exec` calls that write or inspect real files, then a short final sentence. The prompt should avoid exact implementation hints but should require observable artifacts.

This test is specifically aimed at compacted replay with previous `tool_use` and `tool_result` history.

## Machine-Checkable Evidence

- `state> compacting` appears at least 3 times.
- New `shell_exec` calls happen after compact.
- Tool render count increases after each compact.
- No post-compact message says tools are unavailable.
- No old tool result mutates a historical tool call.
- Final workspace artifacts match the prompt.

## Pass Criteria

- At least 3 compact phases.
- At least 3 successful post-compact shell tool continuations.
- The REPL returns to idle after the final cycle.
- No repeated MCP request-id corruption, orphaned tool result, or runaway same-command loop.

## Failure Signals

- Model says `shell_exec` or shell tools are unavailable after compact.
- A current tool result attaches to an old tool call.
- Same command repeats until timeout.
- Compact no-ops because a pending tool remains stuck.

## Process Record

### Run 1

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-compact-haiku-q7wWRJ`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-compact-haiku-q7wWRJ/repl-compact-haiku.log`
- Log size: 2,887,721 bytes.
- Prompt shape: four finite local `shell_exec` pressure turns, each asking for one Python command that writes one marker line plus 720k repeated characters, followed by a no-tool continuation check.
- Process result: exit code 0.
- Compact phases: 4.
- Post-compact tool calls: shell tool executed after compact in pressure turns 2, 3, and 4.
- Final artifacts: pressure markers were observed through REPL shell output; continuation turn completed without shell tools.
- Final counters: `compacting=4`, `shellExec=8`, `usage=5`, `suppressed=0`, `toolUnavailable=0`, `idle=6`.
- Verdict: Passed.

| Step | Compact delta | `shell_exec` render delta | Usage delta | Outcome |
|---|---:|---:|---:|---|
| pressure 1 | 0 | 2 | 1 | Completed without compact; seeded context pressure. |
| pressure 2 | 1 | 2 | 1 | Compact ran before the turn; shell tool executed after compact; completion reply observed. |
| pressure 3 | 1 | 2 | 1 | Second compact; shell tool executed after compact; completion reply observed. |
| pressure 4 | 1 | 2 | 1 | Third compact; shell tool executed after compact; completion reply observed. |
| continuation 5 | 1 | 0 | 1 | Fourth compact; no-tool continuation reply observed. |

## Failure Analysis

No failure remained in the passing run.

Earlier runs exposed real defects before this pass:

- Post-compact replay serialized historical tool names as bare internal names, making the real model believe shell tools were unavailable.
- Repeated MCP request ids could pair a current tool result with an old historical tool call.
- Real model behavior could repeat the same `shell_exec` command until timeout.

Those defects are covered by deterministic provider/agent/shell tests and the passing real run above.

## Follow-Up Deterministic Tests

The passing run is backed by deterministic coverage for provider replay, MCP id uniqueness, transcript tool-pair invariants, and shell loop guard behavior. Future failures should map to the same areas before rerunning this acceptance.
