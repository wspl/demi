# Tool-Heavy Compact Marathon Acceptance Record

| | |
|---|---|
| Date | TBD |
| Status | Designed |
| Scope | Real TUI + real Claude Code provider + shell tools + repeated compact |
| Primary model | `claude-haiku-4-5`, thinking off |
| TUI command | `bun run packages/tui/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 1.00 --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Trigger compact repeatedly while the model must keep using `shell_exec` after compact |

## Scenario Design

Ask for several finite shell-backed benchmark cycles. Each cycle should require a small number of distinct `shell_exec` calls that write or inspect real files, then a short final sentence. The prompt should avoid exact implementation hints but should require observable artifacts.

This test is specifically aimed at compacted replay with previous `tool_use` and `tool_result` history.

## Machine-Checkable Evidence

- `status: compacting` appears at least 3 times.
- New `shell_exec` calls happen after compact.
- Tool render count increases after each compact.
- No post-compact message says tools are unavailable.
- No old tool result mutates a historical tool call.
- Final workspace artifacts match the prompt.

## Pass Criteria

- At least 3 compact phases.
- At least 3 successful post-compact shell tool continuations.
- The TUI returns to idle after the final cycle.
- No repeated MCP request-id corruption, orphaned tool result, or runaway same-command loop.

## Failure Signals

- Model says `shell_exec` or shell tools are unavailable after compact.
- A current tool result attaches to an old tool call.
- Same command repeats until timeout.
- Compact no-ops because a pending tool remains stuck.

## Process Record

### Run 1

- Date: TBD
- Workspace: TBD
- Log path: TBD
- Prompt shape: TBD
- Compact phases: TBD
- Post-compact tool calls: TBD
- Final artifacts: TBD
- Verdict: TBD

## Follow-Up Deterministic Tests

Failures should map to provider replay tests, MCP id uniqueness tests, transcript tool-pair invariants, or shell loop guard tests.
