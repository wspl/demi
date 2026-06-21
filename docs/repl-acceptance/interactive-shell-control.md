# Interactive Shell Control Acceptance Record

| | |
|---|---|
| Date | 2026-06-19 |
| Status | Passed |
| Scope | Real REPL + real Claude Code provider + foreground shell control |
| Primary model | `claude-haiku-4-5`, thinking off |
| REPL command | `/usr/bin/script -q <log> bun run packages/repl/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --yield-after-ms 500 --timeout-ms 120000` |
| Acceptance target | Verify the model can handle commands that wait, ask for input, or run until intentionally aborted |

## Scenario Design

Use a fuzzy task that naturally reaches foreground processes without revealing the control sequence. The prompt asks for a small Node CLI that asks at least two questions, then asks the model to actually run it, enter answers, verify output, start a local long-running service, verify it, and stop it.

Exact prompt used in the final run:

```text
在这个空目录里做一个小的 Node 命令行问答程序：运行时至少询问名字和一个确认问题。你需要实际运行它，按提示输入内容，确认输出正确。然后再启动一个本地长运行服务来证明前台进程控制可用，确认它启动后正常停止。不要只写代码，必须实际验证。
```

## Machine-Checkable Evidence

- `shell_exec` starts finite commands and foreground long-running commands.
- `shell_input` writes non-empty stdin to the foreground process.
- `shell_wait` observes the foreground process without stopping it when only a status poll is intended.
- `shell_exec` without `shellId` can run one-off probes while the default shell has a foreground process.
- `shell_abort` stops the controlled foreground process.
- The workspace contains the expected files under the REPL `--cwd`, not a separate ad hoc `/tmp` project.
- Final independent port probe fails after the model stops the server.
- The REPL returns to idle and exits cleanly.

## Pass Criteria

- The task completes without manual intervention.
- No empty stdin polling.
- No broad `pkill` or process-name killing for controlled foreground processes.
- No accidental `shell_wait timeoutMs` hard stop when the model means to poll.
- No background service restart after the service was already verified and stopped.
- The final REPL process returns to idle and closes on `/exit`.

## Failure Signals

- The model repeatedly sends empty stdin.
- The run gets stuck on an interactive prompt.
- The model kills unrelated processes.
- The foreground process remains active after the task is declared complete.
- The model creates the project outside the REPL workspace.
- The model restarts an already-verified service only to demonstrate the same behavior again.

## Process Record

### Run 1

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-interactive-shell-D9Tahx`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-interactive-shell-D9Tahx/repl-interactive-shell-control.log`
- Log size: 134,815 bytes.
- Result: Failed, then fixed.
- Evidence: service verification worked after curl used auxiliary shells; `shell_abort` stopped the server and an independent port probe failed afterward.
- Failure found: line-oriented stdin was not obvious to the model; it sent input that did not advance the Node readline flow until it rewrote the CLI/verification path.
- Product change: clarify `shell_input` as exact stdin bytes and require newline for line-oriented prompts; add deterministic raw stdin coverage.

### Run 2

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-interactive-shell-tHgX1K`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-interactive-shell-tHgX1K/repl-interactive-shell-control.log`
- Log size: 43,354 bytes.
- Result: Failed, then fixed.
- Evidence: `shell_input` completed the CLI flow smoothly after newline guidance.
- Failure found: the model used `shell_wait timeoutMs` as if it were a poll, which stopped the foreground server before `shell_abort`.
- Product change: document and test that `timeoutMs` is a hard stop; coding-agent prompt tells the model to use `yieldAfterMs` for status polls.

### Run 3

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-interactive-shell-Jk1VL1`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-interactive-shell-Jk1VL1/repl-interactive-shell-control.log`
- Log size: 66,371 bytes.
- Result: Failed, then fixed.
- Evidence: shell control itself was clean: interactive input worked, auxiliary curl probes worked, and `shell_abort` stopped the service.
- Failure found: the model created the project under `/tmp/node-cli-demo` instead of the REPL `--cwd` workspace.
- Product change: coding-agent prompt now treats `cwd` as the task workspace and rejects ad hoc absolute project directories unless the user asks for them.

### Run 4

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-interactive-shell-42FsdL`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-interactive-shell-42FsdL/repl-interactive-shell-control.log`
- Log size: 27,242 bytes.
- Result: Failed, then fixed.
- Evidence: `quiz.js` and `server.js` were created in the correct workspace; interactive stdin and foreground server verification both worked.
- Failure found: after a successful foreground run and abort, the model restarted the server in the background to re-demonstrate the same behavior.
- Product change: coding-agent prompt now tells the model to summarize observed evidence instead of restarting an already-verified long-running process.

### Run 5

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-interactive-shell-Tb2QxZ`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-interactive-shell-Tb2QxZ/repl-interactive-shell-control.log`
- Log size: 111,747 bytes.
- Result: Passed.
- Files created in `--cwd`: `qa.js`, `server.js`, `VERIFICATION_REPORT.md`.
- Input events: `node qa.js` ran as foreground shell `b21cdf6e-7da3-4841-8037-75c0d91f5513`; `shell_input` sent name and confirmation input; the program exited with the expected summary.
- Service events: `node server.js` ran in the foreground; `shell_exec` without `shellId` used auxiliary shells `7c10ca5d-dfbe-4c9e-8cfd-5ef5beacd0a4` and `44b73206-8e06-4849-ab03-bde872db92eb` for curl probes.
- Verification output: `GET /` and `GET /health` were logged by the server; `/health` returned `{"status":"healthy"}`.
- Abort events: `shell_abort` stopped foreground shell `b21cdf6e-7da3-4841-8037-75c0d91f5513`; no background restart was observed.
- Compact behavior: after the model's final summary, REPL auto compacted once, resumed, rechecked `pwd`, `ls`, `qa.js`, and `server.js` in the same workspace, returned to idle, then closed on `/exit`.
- Independent shutdown check: `curl --max-time 2 -sS http://localhost:3000` exited 7 after REPL exit, proving the service was no longer listening.
- Negative checks: no actual `progress> shell[...] timeout`, no `node server.js &`, and no `/tmp/node-cli-demo` in the passing run log.

## Failure Analysis

The passing run is the fifth real REPL attempt. The earlier runs were not discarded because they found product-level friction that deterministic tests alone did not expose:

- The default shell was unusable for one-off probes while a foreground server was running; `shell_exec` without explicit `shellId` now creates an auxiliary shell when the agent default shell is busy.
- The model needed clearer stdin semantics; `shell_input` now documents exact bytes and line-oriented newlines, with raw stdin regression coverage.
- The model treated `timeoutMs` as a poll; tool and prompt text now describe it as a hard stop.
- The model created work outside `--cwd`; coding-agent prompt now establishes the workspace boundary.
- The model repeated an already-verified long-running process; coding-agent prompt now tells it to preserve evidence instead of re-demonstrating.

## Follow-Up Deterministic Tests

The real failures are backed by deterministic coverage in:

- `packages/shell/src/__tests__/environment.test.ts`: busy default shell gets an auxiliary shell; raw stdin without newline keeps a line reader running until newline arrives.
- `packages/shell/src/__tests__/tools.test.ts`: tool descriptions expose auxiliary shell behavior, hard-stop timeout semantics, and newline guidance.
- `packages/coding-agent/src/__tests__/coding-harness.test.ts`: coding-agent prompt locks workspace, polling, abort, no redundant restart, and line-oriented stdin guidance.
- `packages/coding-agent/src/__tests__/coding-marathon.test.ts`: agent-level long-command wait/input/abort flow remains covered.
