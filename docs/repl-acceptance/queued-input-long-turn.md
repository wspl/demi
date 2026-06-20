# Queued Input During Long Turn Acceptance Record

| | |
|---|---|
| Date | 2026-06-19 |
| Status | Passed |
| Scope | Real REPL + AgentClient queue + active long provider turn |
| Primary model | `claude-haiku-4-5`, thinking off |
| REPL command | `/usr/bin/script -q <log> bun run packages/repl/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 1.00 --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Send multiple user inputs while a turn is active and verify ordering, queue display, and drain behavior |

## Scenario Design

Start an active turn that runs a short foreground shell command with `sleep`, then type follow-up A while the active turn is still running. Type follow-up B while A is queued or draining. The final workspace state must prove that ACTIVE, A, and B ran in order.

Final passing prompt sequence:

```text
在这个空目录里启动一个排队顺序检查。先只做 ACTIVE：用 shell 执行 `printf 'ACTIVE_START\n' > order.log; sleep 3; printf 'ACTIVE_END\n' >> order.log`。ACTIVE 完成后只用一句话汇报。请开始执行，不要只描述方案。
```

```text
现在执行后续请求 A（这是 2 个后续请求中的第 1 个）：读取 order.log，确认已有 ACTIVE_END，然后追加 QUEUED_A 到 order.log，创建 queued_a.txt 内容 A_DONE。完成后只用一句话汇报 A 完成；不要执行 B、C 或任何未请求阶段。
```

```text
现在执行后续请求 B（这是 2 个后续请求中的第 2 个，也是最后一个）：读取 order.log，确认 QUEUED_A 位于 ACTIVE_END 之后，然后追加 QUEUED_B 到 order.log，创建 queued_b.txt 内容 B_DONE。完成后只用一句话汇报 B 完成；到此停止，不要执行 C、D 或任何未请求阶段。
```

## Machine-Checkable Evidence

- REPL rendered non-empty queue state while work was active.
- The active turn completed before A's shell mutation.
- A completed before B's shell mutation.
- Final `order.log` contains exactly:
  - `ACTIVE_START`
  - `ACTIVE_END`
  - `QUEUED_A`
  - `QUEUED_B`
- `queued_a.txt` exists with `A_DONE`.
- `queued_b.txt` exists with `B_DONE`.
- The session returned to idle and closed normally.
- Negative checks found no `API Error`, `error> agent`, shell tool error, `state> turn aborted`, `QUEUED_C`, or `QUEUED_D` in the final passing log.

## Pass Criteria

- No queued message is lost.
- No queued message runs before the active turn finishes.
- Final workspace state reflects all requested messages in order.
- The turn converges after B without inventing extra stages.
- REPL returns to idle.

## Failure Signals

- Queue count is wrong or never clears.
- Later message runs before earlier message.
- Only the active send resolves and queued sends hang.
- The model treats queued prompts as future plans instead of current requests.
- The model invents extra queued stages after the requested final stage.

## Process Record

### Run 1

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-queued-input-XXXXXX.SJJ0nbEl6g`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-queued-input-XXXXXX.SJJ0nbEl6g/repl-queued-input-long-turn.log`
- Active prompt: active `sleep 8` order-log prompt.
- Queued prompts: A and B were pasted in one write.
- Queue events: REPL showed `queue> 1 pending`.
- Final artifact ordering: A eventually ran, but B did not drain as a separate queued turn in this run.
- Verdict: Failed harness quality gate. Pasting multiple queued lines in one TTY write was not a clean enough acceptance method.

### Run 2

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-queued-input-XXXXXX.9hekq1RaAq`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-queued-input-XXXXXX.9hekq1RaAq/repl-queued-input-long-turn.log`
- Active prompt: active `sleep 12` order-log prompt.
- Queued prompts: A and B sent as separate input events.
- Queue events: line 43 `queue> 1 pending`, line 57 `queue> 2 pending`, line 118 `queue> 1 pending`.
- Final artifact ordering: final `order.log` only had `ACTIVE_START` and `ACTIVE_END`; no queued files existed.
- Failure signal: the model repeatedly treated the original ACTIVE work as current after queued turns began. One `shell_wait` also timed out during the 12 second sleep.
- Verdict: Failed.

### Run 3

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-queued-input-XXXXXX.ciDscivQEr`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-queued-input-XXXXXX.ciDscivQEr/repl-queued-input-long-turn.log`
- Active prompt: active `sleep 4` order-log prompt with "only process new messages" wording.
- Queued prompts: A and B sent as separate input events.
- Queue events: REPL showed `queue> 1 pending` and `queue> 2 pending`.
- Final artifact ordering: queued turns again failed to focus on A/B and ended without queued artifacts.
- Verdict: Failed.

### Run 4

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-queued-input-XXXXXX.mKllRHdiVT`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-queued-input-XXXXXX.mKllRHdiVT/repl-queued-input-long-turn.log`
- Active prompt: active `sleep 4` prompt.
- Queued prompts: `现在执行 A 阶段...` and `现在执行 B 阶段...`.
- Queue events: REPL showed queued state and A/B drained.
- Final artifact ordering: A and B ran in order, but the model invented `QUEUED_C` and `QUEUED_D` and created `queued_c.txt` / `queued_d.txt`.
- Verdict: Failed convergence gate.

### Run 5

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-queued-input-XXXXXX.n47oB4HC62`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-queued-input-XXXXXX.n47oB4HC62/repl-queued-input-long-turn.log`
- Active prompt: final active prompt above.
- Queued prompts: final A and B prompts above.
- Queue events: line 33 `queue> 1 pending` for A while ACTIVE was running; line 67 `queue> 1 pending` for B while A was draining.
- Final artifact ordering:
  - `order.log` line 1: `ACTIVE_START`
  - `order.log` line 2: `ACTIVE_END`
  - `order.log` line 3: `QUEUED_A`
  - `order.log` line 4: `QUEUED_B`
- Final artifact files:
  - `queued_a.txt=A_DONE`
  - `queued_b.txt=B_DONE`
- Final response evidence: line 111 reports A completion; line 181 reports B completion; line 183 returns `state> idle`.
- Verdict: Passed.

## Failure Analysis

This acceptance test did not require product code changes, but it exposed two real acceptance-design lessons:

- Queue/drain can be correct while the model still misinterprets queued prompts as future planning labels. Prompts that use labels like "后续消息 A" are too easy for the model to treat as a plan instead of the current user request.
- A pattern-like queued workflow can induce the model to invent extra stages after the final requested stage. For this queue acceptance, the final prompt must make the stop condition observable; the separate runaway-tool-loop acceptance should cover uncontrolled continuation.

Existing deterministic queue tests already verify the core runtime contract: queued sends preserve latest user request, drain order, promise resolution, and transcript user-turn order. The real REPL test adds model-facing evidence that the full shell can accept input while active and eventually process queued requests through the real provider path.

## Follow-Up Deterministic Tests

- Existing `packages/agent/src/__tests__/session.test.ts` covers queued sends while active and transcript user-turn order.
- Existing `packages/agent/src/__tests__/session-marathon.test.ts` covers two queued sends plus retry/error recovery.
- Existing `packages/agent/src/__tests__/server.test.ts` covers AgentClient queue frames and queued send promise resolution.
- Existing `packages/repl/src/__tests__/renderer.test.ts` covers queue rendering.
