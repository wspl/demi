# Thinking + Tool Rendering Smoke Acceptance Record

| | |
|---|---|
| Date | 2026-06-19 |
| Status | Passed |
| Scope | Real REPL + real Claude Code provider + thinking + tool output |
| Primary model | `claude-opus-4-8`, medium thinking |
| REPL command | `/usr/bin/script -q <log> bun run packages/repl/src/index.ts --cwd <tmp> --model claude-opus-4-8 --thinking medium --budget 1.00 --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Confirm the REPL shows real model text, thinking output, tool output, and usage without duplicates |

## Scenario Design

Run a short shell-inspection task in a fresh temporary workspace. The prompt requires the agent to create `smoke.txt`, read it back, count bytes, and summarize only the verified result:

```text
在这个空目录里做一个简短的真实工具调用检查：用 shell 命令创建 smoke.txt，内容包含 DEMI_THINKING_TOOL_SMOKE 和当前工作目录；再用 shell 命令读取 smoke.txt 并统计字节数。最后用一句中文总结你实际验证到的文件内容和字节数。请开始执行，不要只描述方案。
```

## Machine-Checkable Evidence

- The REPL header shows `model     claude-opus-4-8`.
- The REPL header shows `thinking  medium`.
- The REPL log contains `thinking> [signed]` from the real provider.
- The REPL log contains `tool> shell_exec executing` and `tool> shell_exec completed`.
- Shell output includes `DEMI_THINKING_TOOL_SMOKE <workspace>` and `116 smoke.txt`.
- The assistant final text reports the actual file content and byte count.
- The REPL log contains `usage> in=53 out=252 cache_read=3755 cache_write=4070`.
- The session closed normally with `/exit`.
- Negative checks found no `API Error`, `error> agent`, `tool> shell_exec error`, or `state> turn aborted`.

## Pass Criteria

- Real provider response is observed.
- Thinking, assistant text, tool output, and usage all render.
- The task completes and returns to idle before exit.
- The model id and thinking effort match the command.

## Failure Signals

- Thinking is missing when the provider emits it.
- Text deltas duplicate.
- Tool output appears only in transcript but not REPL.
- Wrong model or thinking effort is used.

## Process Record

### Run 1

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-thinking-smoke-XXXXXX.XiUiC9ql5B`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-thinking-smoke-XXXXXX.XiUiC9ql5B/repl-thinking-tool-rendering-smoke.log`
- Prompt: smoke prompt above.
- Thinking evidence: line 17 `thinking> [signed]`.
- Tool output evidence: lines 18 and 23 show `shell_exec` executing/completed; lines 20 and 21 show the sentinel content and byte count.
- Usage evidence: line 26 `usage> in=53 out=252 cache_read=3755 cache_write=4070`.
- Workspace evidence: `wc -c smoke.txt` returned `116`, and the first line was `DEMI_THINKING_TOOL_SMOKE /var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-repl-thinking-smoke-XXXXXX.XiUiC9ql5B`.
- Verdict: Passed.

## Failure Analysis

No product bug was found in this run. It confirms the real REPL path can display a real Claude Code provider response with medium thinking, shell tool execution/output, assistant text, usage fields, and normal close behavior.

## Follow-Up Deterministic Tests

- Existing `packages/provider-claude-code/src/__tests__/jsonl-output.test.ts` covers Claude thinking/text/tool/usage event mapping.
- Existing `packages/repl/src/__tests__/renderer.test.ts` covers renderer delta de-duplication for thinking/text/tool/usage.
- Existing `packages/repl/src/__tests__/real-repl.e2e.test.ts` keeps an opt-in gated smoke for repeated real REPL `claude-opus-4-8` / medium runs.
