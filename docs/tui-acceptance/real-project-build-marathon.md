# Real Project Build Marathon Acceptance Record

| | |
|---|---|
| Date | TBD |
| Status | Designed |
| Scope | Real TUI + real Claude Code provider + real frontend project |
| Primary model | `claude-haiku-4-5` for cost runs; optionally `claude-opus-4-8` medium thinking for higher-fidelity runs |
| TUI command | `bun run packages/tui/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 2.00 --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Complete a fuzzy Vue + Pinia todo list task end to end and verify real project commands pass |

## Scenario Design

Use a deliberately imperfect user request, for example: initialize a Vue project and write a Vue + Pinia todo list. Do not include the exact scaffold flags or implementation. The model should choose non-interactive defaults, create files, run install/build/test or equivalent verification, and fix errors.

## Machine-Checkable Evidence

- Project files are created in the workspace.
- A Pinia store exists and is used by the app.
- Todo list supports add, toggle, delete, and filter or persistence if the model chooses it.
- Build or test command exits successfully.
- TUI shows real tool output and returns to idle.

## Pass Criteria

- The generated app builds successfully.
- The requested Vue + Pinia todo list exists in code, not only in prose.
- The model handles scaffold prompts without manual rescue.
- No stuck long process remains after the run.

## Failure Signals

- The model only writes a plan.
- Scaffold gets stuck on interactive prompts.
- The model uses destructive workspace commands.
- Build fails and the model claims success.
- Dev server or watcher remains running after completion.

## Process Record

### Run 1

- Date: TBD
- Workspace: TBD
- Log path: TBD
- Prompt: TBD
- Commands observed: TBD
- Created files: TBD
- Verification command and exit code: TBD
- Verdict: TBD

## Follow-Up Deterministic Tests

Failures should become `agent-coding` scenario tests, shell foreground-control tests, or TUI process tests depending on the broken layer.
