# TUI Long-Run Acceptance Suite

This document lists the long-running real TUI acceptance tests we should use to validate the agent shell. Each test has its own process record under `docs/tui-acceptance/`.

The purpose is to catch failures that deterministic unit tests usually miss: model drift, provider event mismatch, shell control friction, context pressure, compaction recovery, and UI/RPC action convergence.

## Run Discipline

- Treat these as gated acceptance tests, not default unit tests.
- Use a fresh temporary workspace for every run.
- Record the exact TUI command, model, budget, prompt shape, workspace, log path, and final verdict.
- Prefer machine-checkable signals over model self-report: phase sequence, `usage:` lines, compact count, tool render count, output sentinels, real files, build/test exit code, and known error keywords.
- When a run exposes a product bug, add a deterministic regression test for the fixed contract before closing the acceptance record.
- Do not rewrite the prompt to reveal the intended solution. The prompt should exercise fuzzy, realistic model behavior.

## Test Matrix

| Test | Priority | Main capability | Process record |
|---|---|---|---|
| Auto Compact Marathon | P0 | Provider-usage auto compact, cache pressure, post-compact resume | [auto-compact-marathon.md](tui-acceptance/auto-compact-marathon.md) |
| Tool-Heavy Compact Marathon | P0 | Shell tool replay after compact, MCP ids, tool pairing | [tool-heavy-compact-marathon.md](tui-acceptance/tool-heavy-compact-marathon.md) |
| Real Project Build Marathon | P0 | End-to-end coding workflow under fuzzy requirements | [real-project-build-marathon.md](tui-acceptance/real-project-build-marathon.md) |
| Interactive Shell Control | P0 | Foreground process, wait/input/abort, scaffold prompts | [interactive-shell-control.md](tui-acceptance/interactive-shell-control.md) |
| Compact + Abort + Resume | P0 | Abort/compact/resume interleaving and transcript atomicity | [compact-abort-resume.md](tui-acceptance/compact-abort-resume.md) |
| Queued Input During Long Turn | P1 | TUI/RPC queue ordering while a turn is active | [queued-input-long-turn.md](tui-acceptance/queued-input-long-turn.md) |
| Context Cache Stability Marathon | P1 | Cache usage, stable prefix, post-compact restabilization | [context-cache-stability-marathon.md](tui-acceptance/context-cache-stability-marathon.md) |
| Large Output Boundaries | P1 | Model-visible truncation and UI high-output stability | [large-output-boundaries.md](tui-acceptance/large-output-boundaries.md) |
| Thinking + Tool Rendering Smoke | P1 | Real thinking, real text, real tool output rendering | [thinking-tool-rendering-smoke.md](tui-acceptance/thinking-tool-rendering-smoke.md) |
| Runaway Tool Loop Guard | P1 | Repeated command suppression and turn convergence | [runaway-tool-loop-guard.md](tui-acceptance/runaway-tool-loop-guard.md) |

## Shared Evidence Format

Every process record should include:

- `Date`
- `Status`: `Designed`, `Running`, `Passed`, `Failed`, or `Superseded`
- `Scope`
- `TUI command`
- `Model and thinking setting`
- `Workspace`
- `Log path`
- `Prompt shape`
- `Acceptance target`
- `Machine-checkable evidence`
- `Runs`
- `Failure analysis`
- `Follow-up deterministic tests`

## Current Status

- `docs/tui-compact-haiku-acceptance.md` is the first completed long-run record.
- `docs/tui-acceptance/auto-compact-marathon.md` is passed and has been backfilled from the completed real TUI run.
- `docs/tui-acceptance/tool-heavy-compact-marathon.md` is passed and has been backfilled from the completed real TUI run.
- `docs/tui-acceptance/real-project-build-marathon.md` is passed from a real Vue + Pinia project build run.
- `docs/tui-acceptance/interactive-shell-control.md` is passed after five real TUI runs; the first four runs exposed shell-control friction that now has deterministic regression coverage.
- Other records in `docs/tui-acceptance/` remain designs until they are executed and updated with real logs.
