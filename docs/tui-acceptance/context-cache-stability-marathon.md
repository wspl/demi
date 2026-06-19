# Context Cache Stability Marathon Acceptance Record

| | |
|---|---|
| Date | TBD |
| Status | Designed |
| Scope | Real TUI + real Claude Code provider + repeated stable-prefix turns |
| Primary model | `claude-haiku-4-5`, thinking off |
| TUI command | `bun run packages/tui/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 1.00 --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Verify cache usage appears, compact resets context pressure, and stable-prefix behavior resumes after compact |

## Scenario Design

Run many similar turns with stable instructions and small observable outputs. Include at least one compact point. Avoid changing tools/system prompt/model within the session.

## Machine-Checkable Evidence

- `usage:` lines include non-zero cache fields when the provider reports them.
- Cache-backed usage grows over repeated stable-prefix turns.
- After compact, usage drops and then grows again.
- Model-visible output does not include raw cache metadata.

## Pass Criteria

- Cache fields are preserved through TUI rendering.
- Compact does not permanently destabilize provider request prefix behavior.
- The agent continues normal turns after compact.

## Failure Signals

- Cache usage fields disappear from TUI output.
- Cache pressure grows past threshold without compact.
- After compact, every turn behaves like a cold context indefinitely.
- Cache metadata leaks into assistant-visible or user-visible content.

## Process Record

### Run 1

- Date: TBD
- Workspace: TBD
- Log path: TBD
- Prompt sequence: TBD
- Cache usage sequence: TBD
- Compact events: TBD
- Verdict: TBD

## Follow-Up Deterministic Tests

Failures should update provider usage mapping tests, TUI renderer tests, or context-cache stable prefix tests.
