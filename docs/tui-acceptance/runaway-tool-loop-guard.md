# Runaway Tool Loop Guard Acceptance Record

| | |
|---|---|
| Date | TBD |
| Status | Designed |
| Scope | Real TUI + real Claude Code provider + repeated tool behavior |
| Primary model | `claude-haiku-4-5`, thinking off |
| TUI command | `bun run packages/tui/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 1.00 --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Verify repeated identical shell commands converge instead of flooding the session |

## Scenario Design

Use a task that may tempt the model to retry the same command repeatedly, such as waiting for a sentinel that will not change, checking a fixed file, or running a command whose output already answers the question. The prompt should be realistic, not adversarial.

## Machine-Checkable Evidence

- Identical rapid `shell_exec` scripts are counted.
- Suppression or terminal tool result appears when the repeated threshold is crossed.
- The turn returns to idle instead of running until timeout.
- The model can continue with a new user turn afterward.

## Pass Criteria

- No unbounded repeated command loop.
- TUI output volume remains bounded.
- The session stays reusable after suppression or convergence.

## Failure Signals

- Same command runs many times until timeout.
- Tool suppression fires but leaves the turn stuck.
- New user input after the loop cannot run.
- Suppression blocks legitimate distinct commands.

## Process Record

### Run 1

- Date: TBD
- Workspace: TBD
- Log path: TBD
- Prompt: TBD
- Repeated command count: TBD
- Suppression evidence: TBD
- Post-loop continuation: TBD
- Verdict: TBD

## Follow-Up Deterministic Tests

Failures should map to shell loop guard tests, terminal tool result handling, or AgentSession continuation behavior.
