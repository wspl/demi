# Compact + Abort + Resume Acceptance Record

| | |
|---|---|
| Date | TBD |
| Status | Designed |
| Scope | Real TUI + real Claude Code provider + abort/compact/resume interleaving |
| Primary model | `claude-haiku-4-5`, thinking off |
| TUI command | `bun run packages/tui/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 1.00 --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Abort a pressured long turn, then resume or continue without corrupting transcript state |

## Scenario Design

Start a turn that produces a large amount of tool history or model-visible text. Abort while it is active, then use `/resume` or send a follow-up task that requires the agent to continue from the useful partial state.

## Machine-Checkable Evidence

- `/abort` returns the TUI to idle.
- Pending tool calls are completed as error results or cleared according to session rules.
- A later compact does not leave half-written boundary or marker blocks.
- Resume or follow-up request can call tools and produce a normal response.

## Pass Criteria

- Abort converges to idle.
- Resume or follow-up completes.
- No orphaned `tool_use` or `tool_result`.
- No duplicate execution of completed tools after resume.

## Failure Signals

- Session stays busy after abort.
- Resume fails because a pending tool is stuck.
- Compact summary writes a boundary without a marker, or vice versa.
- The model loses essential partial progress that should have been summarized.

## Process Record

### Run 1

- Date: TBD
- Workspace: TBD
- Log path: TBD
- Prompt: TBD
- Abort timing: TBD
- Resume path: TBD
- Compact phases: TBD
- Verdict: TBD

## Follow-Up Deterministic Tests

Failures should map to `AgentSession` abort, compaction atomicity, pending-tool completion, or resume request shape tests.
