# Queued Input During Long Turn Acceptance Record

| | |
|---|---|
| Date | TBD |
| Status | Designed |
| Scope | Real TUI + RPC queue + active long provider turn |
| Primary model | `claude-haiku-4-5`, thinking off |
| TUI command | `bun run packages/tui/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 1.00 --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Send multiple user inputs while a turn is active and verify ordering, queue display, and drain behavior |

## Scenario Design

Start a long-running task that takes enough time to accept more user input. While it is running, send two follow-up messages that depend on order. The prompt should require observable final state for each queued message.

## Machine-Checkable Evidence

- TUI renders non-empty queue state.
- First turn completes before queued sends are processed.
- Queued messages drain in send order.
- Each queued message has corresponding transcript and provider output.
- Promise/action resolution does not cross-wire errors between turns.

## Pass Criteria

- No queued message is lost.
- No queued message runs before the active turn finishes or aborts.
- Final workspace state reflects all messages in order.
- TUI returns to idle.

## Failure Signals

- Queue count is wrong or never clears.
- Later message runs before earlier message.
- Only the active send resolves and queued sends hang.
- Provider error from one queued send rejects the wrong user action.

## Process Record

### Run 1

- Date: TBD
- Workspace: TBD
- Log path: TBD
- Active prompt: TBD
- Queued prompts: TBD
- Queue events: TBD
- Final artifact ordering: TBD
- Verdict: TBD

## Follow-Up Deterministic Tests

Failures should map to RPC queue tests, `AgentSession` pending action drain tests, or TUI input loop tests.
