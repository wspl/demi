# Interactive Shell Control Acceptance Record

| | |
|---|---|
| Date | TBD |
| Status | Designed |
| Scope | Real TUI + real Claude Code provider + foreground shell control |
| Primary model | `claude-haiku-4-5`, thinking off |
| TUI command | `bun run packages/tui/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 1.00 --yield-after-ms 500 --timeout-ms 120000` |
| Acceptance target | Verify the model can handle commands that wait, ask for input, or run until aborted |

## Scenario Design

Give a fuzzy task that naturally reaches foreground processes: a scaffold command, a small CLI that asks for a value, a dev server, or a watcher. The prompt should not reveal the exact shell-control sequence.

## Machine-Checkable Evidence

- Long-running commands return `status=running`.
- The model uses wait/poll behavior instead of sending empty input.
- Non-empty stdin is sent only when the foreground process is actually waiting for it.
- The model stops controlled foreground processes through intentional abort behavior.
- Final shell state is idle and reusable.

## Pass Criteria

- The task completes without manual intervention.
- No empty stdin polling.
- No broad `pkill` or process-name killing for controlled foreground processes.
- The TUI returns to idle.

## Failure Signals

- The model repeatedly sends empty stdin.
- The run gets stuck on an interactive prompt.
- The model kills unrelated processes.
- The foreground process remains active after the task is declared complete.

## Process Record

### Run 1

- Date: TBD
- Workspace: TBD
- Log path: TBD
- Prompt: TBD
- Foreground commands: TBD
- Input events: TBD
- Abort events: TBD
- Verdict: TBD

## Follow-Up Deterministic Tests

Failures should map to shell `wait/input/abort` tests, TUI slash input tests, or coding-agent system prompt changes.
