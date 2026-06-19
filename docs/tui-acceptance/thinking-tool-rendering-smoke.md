# Thinking + Tool Rendering Smoke Acceptance Record

| | |
|---|---|
| Date | TBD |
| Status | Designed |
| Scope | Real TUI + real Claude Code provider + thinking + tool output |
| Primary model | `claude-opus-4-8`, medium thinking |
| TUI command | `bun run packages/tui/src/index.ts --cwd <tmp> --model claude-opus-4-8 --thinking medium --budget 1.00 --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Confirm the TUI shows real model text, thinking output, tool output, and usage without duplicates |

## Scenario Design

Use a short coding or shell-inspection task that naturally calls one or two shell tools. The prompt should not mention rendering internals. The run is costly enough to stay gated.

## Machine-Checkable Evidence

- TUI shows `thinking>` content from the real provider.
- TUI shows `assistant>` final text from the real model.
- TUI shows tool output and final `usage:` fields.
- Duplicate thinking/text deltas are not printed.
- The model id and thinking effort match the command.

## Pass Criteria

- Real provider response is observed.
- Thinking, assistant text, tool output, and usage all render once.
- The task completes and returns to idle.

## Failure Signals

- Thinking is missing when the provider emits it.
- Text deltas duplicate.
- Tool output appears only in transcript but not TUI.
- Wrong model or thinking effort is used.

## Process Record

### Run 1

- Date: TBD
- Workspace: TBD
- Log path: TBD
- Prompt: TBD
- Thinking evidence: TBD
- Tool output evidence: TBD
- Usage evidence: TBD
- Verdict: TBD

## Follow-Up Deterministic Tests

Failures should update Claude Code event mapping tests, TUI renderer tests, or real provider gated smoke tests.
