# Runaway Tool Loop Guard Acceptance Record

| | |
|---|---|
| Date | 2026-06-19 |
| Status | Passed |
| Scope | Real TUI + real Claude Code provider + repeated tool behavior |
| Primary model | `claude-haiku-4-5`, thinking off |
| TUI command | `/usr/bin/script -q <log> bun run packages/tui/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 1.00 --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Verify repeated identical shell commands are bounded, visible in TUI, and do not make the session unusable afterward |

## Scenario Design

Use a task that may tempt the model to retry the same command repeatedly, such as waiting for a sentinel that will not change, checking a fixed file, or running a command whose output already answers the question. The prompt should be realistic, not adversarial.

## Machine-Checkable Evidence

- Identical rapid `shell_exec` scripts are counted and suppressed at the configured threshold.
- TUI renders the suppression output, including the reason and suggested recovery.
- The provider receives the suppression tool result and can continue the current turn.
- The turn returns to idle instead of running until timeout.
- A new user turn after suppression can run a different shell command.
- The sentinel file remains unchanged.

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

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-tui-runaway-loop-XXXXXX.xqKCesdLXu`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-tui-runaway-loop-XXXXXX.xqKCesdLXu/tui-runaway-tool-loop-guard.log`
- Prompt: create `sentinel.txt` with `WAITING`, poll it with independent `cat sentinel.txt` commands up to 10 times, and stop if it never becomes `READY`.
- Verdict: Failed.
- Failure signature: the repeated shell guard fired, but the TUI only displayed `tool: shell_exec error cat sentinel.txt`; it did not display the suppression details.
- Product gap found: the TUI renderer ignored output on `tool_call` blocks whose status was `error`, so tool-layer recovery guidance was invisible to the operator.
- Fix added: TUI now renders `tool error>` output lines for errored tool calls and de-duplicates them across transcript patch replay.
- Regression test added: `packages/tui/src/__tests__/renderer.test.ts` asserts the repeated-shell suppression message is rendered exactly once.

### Run 2

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-tui-runaway-loop-XXXXXX.irSOq01B9r`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-tui-runaway-loop-XXXXXX.irSOq01B9r/tui-runaway-tool-loop-guard.log`
- Prompt: same sentinel polling prompt as Run 1.
- Verdict: Failed.
- Runtime evidence: TUI displayed `Repeated identical shell_exec suppressed.` and the threshold line, but a follow-up `wc -c sentinel.txt` request was swallowed by the old pending Claude Code control request and the model tried `cat sentinel.txt` again.
- Product gap found: repeated-shell suppression returned `stopAfterToolResult: true`. That ended the `AgentSession` turn before the active Claude Code control request received the tool result, so a subsequent user message could race with an old provider continuation.
- Fix added: repeated-shell suppression is now a normal tool result. The provider sees the suppression result, decides how to recover or stop, and the session only accepts the next user message after that turn has converged.
- Regression test added: `packages/shell/src/__tests__/tools.test.ts` covers both the direct suppression result contract and an `AgentSession` integration where a new send works after suppression.

### Run 3

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-tui-runaway-loop-VXAKHS`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-tui-runaway-loop-VXAKHS/tui-runaway-tool-loop-guard.log`
- Prompt: same sentinel polling prompt as Run 1.
- Follow-up prompt: run `wc -c sentinel.txt`, answer only the byte count, and do not run `cat sentinel.txt`.
- Repeated command count: `cat sentinel.txt` reached the suppression threshold after 7 identical calls; the model then used distinct commands to complete the requested maximum of 10 checks.
- Verdict: Passed.

Key final-pass log evidence:

- Suppression: line 100 `tool: shell_exec error cat sentinel.txt`.
- TUI error details: lines 101-103 print `Repeated identical shell_exec suppressed.`, `The same script has been run 7 consecutive times in this agent session.`, and the recovery instruction.
- Current turn convergence: line 131 reports the experiment ended; line 135 `status: idle`.
- New turn usability: line 147 executes `wc -c sentinel.txt`; line 151 completes it; line 155 assistant answers `8`; line 157 `status: idle`.
- Final close: line 159 `closed`.

Workspace evidence:

- `sentinel.txt` stayed `WAITING`.
- `wc -c sentinel.txt` returned `8`.

Negative checks on the final passing log:

- `API Error`: absent.
- `agent error`: absent.
- `turn aborted`: absent.
- unexpected shell error during the follow-up `wc`: absent.

Non-product note: a failed intermediate run's outer `script` wrapper did not exit immediately after `/exit`; `Ctrl-C` in that PTY closed it. The final passing TUI session exited cleanly with `/exit`.

## Failure Analysis

This acceptance test found two bugs that deterministic tests had missed:

- Tool errors can carry actionable output. The TUI must render that output or the operator cannot understand why the agent stopped or changed strategy.
- A guard result that is meant to steer the model must not be treated as a terminal session stop. The active provider continuation needs to receive the tool result before new user input is accepted.

The final run confirms the desired behavior: repeated identical commands are bounded, the suppression is visible, the model can converge, and the same session can execute a different follow-up command.

## Follow-Up Deterministic Tests

- `packages/tui/src/__tests__/renderer.test.ts`: errored tool-call output is rendered with `tool error>` and not duplicated on patch replay.
- `packages/shell/src/__tests__/tools.test.ts`: repeated `shell_exec` suppression returns a non-terminal tool result.
- `packages/shell/src/__tests__/tools.test.ts`: `AgentSession` can receive a repeated-command suppression result, complete the turn, and accept a later user send.
