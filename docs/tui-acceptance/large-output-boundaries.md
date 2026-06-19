# Large Output Boundaries Acceptance Record

| | |
|---|---|
| Date | TBD |
| Status | Designed |
| Scope | Real TUI + shell output + transcript bounding |
| Primary model | `claude-haiku-4-5`, thinking off |
| TUI command | `bun run packages/tui/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 1.00 --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Verify large command output is rendered safely, bounded in model context, and handled honestly by the model |

## Scenario Design

Ask the agent to inspect or generate a large log or file with sentinel lines at the head, middle, and tail. Then ask it to answer a question that can only be answered from visible bounded content or from an explicit follow-up command.

## Machine-Checkable Evidence

- TUI renders high-volume output without losing sentinels needed by the harness.
- Provider-visible transcript uses head/tail truncation for oversized tool output.
- The model does not claim to know omitted middle content unless it runs another command.
- Follow-up commands can inspect missing ranges when needed.

## Pass Criteria

- TUI remains responsive and returns to idle.
- Output bounding does not corrupt shell session state.
- The model either uses available head/tail evidence or explicitly inspects the missing region.

## Failure Signals

- TUI drops sentinel output or locks up.
- Model-visible context contains the full huge output.
- The model hallucinates omitted middle content.
- Shell output truncation hides needed diagnostics without a way to recover.

## Process Record

### Run 1

- Date: TBD
- Workspace: TBD
- Log path: TBD
- Prompt: TBD
- Output size: TBD
- Sentinel observations: TBD
- Model answer check: TBD
- Verdict: TBD

## Follow-Up Deterministic Tests

Failures should map to TUI high-output tests, transcript bounding tests, or shell output snapshot tests.
