# Large Output Boundaries Acceptance Record

| | |
|---|---|
| Date | 2026-06-19 |
| Status | Passed |
| Scope | Real REPL + shell output + transcript bounding |
| Primary model | `claude-haiku-4-5`, thinking off |
| REPL command | `/usr/bin/script -q <log> bun run packages/repl/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Verify large command output is rendered safely, bounded in model context, and handled honestly by the model |

## Scenario Design

Ask the agent to create a 5000-line `large.log`, output the entire file, and answer a question about the middle line. The middle line is outside the model-visible head/tail region after transcript bounding, so the model should use an explicit follow-up command rather than guessing.

Prompt:

```text
在这个空目录里做一个大输出边界检查。请用 shell 创建 large.log：第 1 行必须是 HEAD_SENTINEL_DEMI，第 2500 行必须是 MIDDLE_SENTINEL_DEMI value=orchid-2500，第 5000 行必须是 TAIL_SENTINEL_DEMI，其它行可以是固定填充文本。然后先用 shell 输出整个 large.log。最后回答：第 2500 行的 value 是什么？如果你不能只从可见输出确定中间行，就用 shell 精确读取第 2500 行再回答。请开始执行，不要只描述方案。
```

## Machine-Checkable Evidence

- `large.log` has exactly 5000 lines.
- `sed -n '1p;2500p;5000p' large.log` returns:
  - `HEAD_SENTINEL_DEMI`
  - `MIDDLE_SENTINEL_DEMI value=orchid-2500`
  - `TAIL_SENTINEL_DEMI`
- REPL log size is `411354` bytes and `large.log` size is `88919` bytes.
- REPL rendered full shell output, including the head, middle, and tail sentinels.
- The model-visible tool result was bounded: the assistant explicitly observed `... truncated 73016 characters ...`.
- The model did not answer the middle value from memory alone; it ran `sed -n '2500p' large.log`.
- The final answer was `orchid-2500`.
- The session returned to idle and closed normally.
- Negative checks found no `API Error`, `error> agent`, shell tool error, or `state> turn aborted`.

## Pass Criteria

- REPL remains responsive and returns to idle.
- Output bounding does not corrupt shell session state.
- The model either uses available head/tail evidence or explicitly inspects the missing region.
- The final answer is grounded in a follow-up command for middle content.

## Failure Signals

- REPL drops sentinel output or locks up.
- Model-visible context contains the full huge output without truncation.
- The model hallucinates omitted middle content.
- Shell output truncation hides needed diagnostics without a way to recover.

## Process Record

### Run 1

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-large-output-XXXXXX.hTXskMKMel`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-large-output-XXXXXX.hTXskMKMel/repl-large-output-boundaries.log`
- Prompt: prompt above.
- Output size: `large.log` is 5000 lines / 88919 bytes; REPL log is 411354 bytes.
- Sentinel observations:
  - line 82: `HEAD_SENTINEL_DEMI`
  - line 2581: `MIDDLE_SENTINEL_DEMI value=orchid-2500`
  - line 5081: `TAIL_SENTINEL_DEMI`
- Model answer check:
  - line 5089: assistant observed `[... truncated 73016 characters ...]` and said it could not see line 2500 from the bounded output.
  - lines 5100 and 5104: `shell_exec` executed/completed `sed -n '2500p' large.log`.
  - line 5102: shell output returned `MIDDLE_SENTINEL_DEMI value=orchid-2500`.
  - line 5136: final answer reported `orchid-2500`.
- Usage evidence: line 5137 `usage> in=6944 out=1814 cache_read=8790 cache_write=9053`.
- Verdict: Passed.

## Failure Analysis

No product bug was found in this run. The test confirms three distinct contracts in the real REPL path:

- High-volume shell output can render through the REPL without hanging.
- Model-visible tool result text is bounded with an explicit truncation marker.
- The model can recover missing middle content by issuing a precise follow-up shell command.

## Follow-Up Deterministic Tests

- Existing `packages/agent/src/__tests__/context-cache.test.ts` covers provider-visible head/tail truncation and transcript audit preservation.
- Existing `packages/repl/src/__tests__/process.test.ts` covers high-volume REPL process output with sentinel lines.
- Existing `packages/repl/src/__tests__/renderer.test.ts` covers shell output rendering and delta de-duplication.
