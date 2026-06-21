# REPL Long-Run Acceptance Suite

This document lists the long-running real REPL acceptance tests we should use to validate the agent shell. Each test has its own process record under `docs/repl-acceptance/`.

The purpose is to catch failures that deterministic unit tests usually miss: model drift, provider event mismatch, shell control friction, context pressure, compaction recovery, and UI/AgentClient action convergence.

## Run Discipline

- Treat these as gated acceptance tests, not default unit tests.
- Use a fresh temporary workspace for every run.
- Record the exact REPL command, model, prompt shape, workspace, log path, and final verdict.
- Prefer machine-checkable signals over model self-report: phase sequence, `usage>` lines, compact count, tool render count, output sentinels, real files, build/test exit code, and known error keywords.
- When a run exposes a product bug, add a deterministic regression test for the fixed contract before closing the acceptance record.
- Do not rewrite the prompt to reveal the intended solution. The prompt should exercise fuzzy, realistic model behavior.

## Test Matrix

| Test | Priority | Main capability | Process record |
|---|---|---|---|
| Auto Compact Marathon | P0 | Provider-usage auto compact, cache pressure, post-compact resume | [auto-compact-marathon.md](repl-acceptance/auto-compact-marathon.md) |
| Tool-Heavy Compact Marathon | P0 | Shell tool replay after compact, MCP ids, tool pairing | [tool-heavy-compact-marathon.md](repl-acceptance/tool-heavy-compact-marathon.md) |
| Real Project Build Marathon | P0 | End-to-end coding workflow under fuzzy requirements | [real-project-build-marathon.md](repl-acceptance/real-project-build-marathon.md) |
| Interactive Shell Control | P0 | Foreground process, wait/input/abort, scaffold prompts | [interactive-shell-control.md](repl-acceptance/interactive-shell-control.md) |
| Compact + Abort + Resume | P0 | Abort/compact/resume interleaving and transcript atomicity | [compact-abort-resume.md](repl-acceptance/compact-abort-resume.md) |
| Queued Input During Long Turn | P1 | REPL/AgentClient queue ordering while a turn is active | [queued-input-long-turn.md](repl-acceptance/queued-input-long-turn.md) |
| Context Cache Stability Marathon | P1 | Cache usage, stable prefix, post-compact restabilization | [context-cache-stability-marathon.md](repl-acceptance/context-cache-stability-marathon.md) |
| Large Output Boundaries | P1 | Model-visible truncation and UI high-output stability | [large-output-boundaries.md](repl-acceptance/large-output-boundaries.md) |
| Thinking + Tool Rendering Smoke | P1 | Real thinking, real text, real tool output rendering | [thinking-tool-rendering-smoke.md](repl-acceptance/thinking-tool-rendering-smoke.md) |
| Runaway Tool Loop Guard | P1 | Repeated command suppression and turn convergence | [runaway-tool-loop-guard.md](repl-acceptance/runaway-tool-loop-guard.md) |

## Shared Evidence Format

Every process record should include:

- `Date`
- `Status`: `Designed`, `Running`, `Passed`, `Failed`, or `Superseded`
- `Scope`
- `REPL command`
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

- `docs/repl-compact-haiku-acceptance.md` is the first completed long-run record.
- `docs/repl-acceptance/auto-compact-marathon.md` is passed and has been backfilled from the completed real REPL run.
- `docs/repl-acceptance/tool-heavy-compact-marathon.md` is passed and has been backfilled from the completed real REPL run.
- `docs/repl-acceptance/real-project-build-marathon.md` is passed from a real Vue + Pinia project build run.
- `docs/repl-acceptance/interactive-shell-control.md` is passed after five real REPL runs; the first four runs exposed shell-control friction that now has deterministic regression coverage.
- `docs/repl-acceptance/compact-abort-resume.md` is passed after three real REPL runs; the first two exposed unsigned-thinking replay and repeated abort rendering gaps now covered by deterministic tests.
- `docs/repl-acceptance/thinking-tool-rendering-smoke.md` is passed with `claude-opus-4-8` and medium thinking; it verifies real thinking, real shell output, and real model text.
- `docs/repl-acceptance/queued-input-long-turn.md` is passed after multiple real runs; it records queue-ordering failures before the final prompt shape converged.
- `docs/repl-acceptance/large-output-boundaries.md` is passed; it verifies large shell output remains visible to the UI while the model recovers by precisely rereading the hidden middle line.
- `docs/repl-acceptance/runaway-tool-loop-guard.md` is passed after exposing two product gaps: errored tool output was hidden in REPL, and repeated-shell suppression incorrectly stopped the session before provider continuation consumed the tool result. Both now have deterministic regression tests.
- `docs/repl-acceptance/context-cache-stability-marathon.md` is passed; it records that short one-tool turns may report zero cache usage, while real multi-tool provider continuations report cache hits before and after manual compact.
- All current P0 and P1 records in `docs/repl-acceptance/` have real process records.
