# Compact + Abort + Resume Acceptance Record

| | |
|---|---|
| Date | 2026-06-19 |
| Status | Passed |
| Scope | Real TUI + real Claude Code provider + abort/resume/auto compact/manual compact interleaving |
| Primary model | `claude-haiku-4-5`, thinking off |
| TUI command | `/usr/bin/script -q <log> bun run packages/tui/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 1.00 --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Abort a long tool turn, resume from the transcript, compact, and keep using tools without provider replay or TUI rendering corruption |

## Scenario Design

Run the TUI in a fresh temporary workspace and send this fuzzy long-task prompt:

```text
在这个空目录里做一个可恢复的长任务实验：创建 progress.log 和 artifacts/。分阶段完成 12 个 checkpoint（CHECKPOINT_01 到 CHECKPOINT_12），每个 checkpoint 都必须用 shell 命令追加到 progress.log，并写一个对应的 artifacts/checkpoint_XX.txt。每阶段要实际读取或确认前一阶段文件；如果中途被中断，之后继续时先检查已有 checkpoint，不要重做已完成项，只补缺失项。为了便于观察长 turn，前几阶段可以用短 sleep 或批量输出制造一点等待。最后写 SUMMARY.md，列出实际完成的 checkpoint 和验证命令。请开始执行，不要只描述方案。
```

The operator sends `/abort` while the first turn is active, then `/resume`. After the resumed task completes, the operator waits for usage-based auto compact, sends `/compact`, then asks for a shell-based follow-up verification.

## Machine-Checkable Evidence

- `/abort` returned to idle and emitted exactly one `turn aborted` line in the final passing log.
- `/resume` after abort started a new running turn and did not produce `API Error`, `thinking.signature`, or `agent error`.
- The resumed model inspected existing `progress.log` / `artifacts/`, found no completed checkpoint files after the early abort, then completed `CHECKPOINT_01` through `CHECKPOINT_12`.
- The workspace contained exactly 12 `artifacts/checkpoint_*.txt` files.
- `SUMMARY.md` existed and the post-compact follow-up read its first line: `# Long Task Recovery Experiment - Summary Report`.
- Usage-based auto compact occurred after the long resumed turn, then the TUI returned to running and the model successfully called `shell_exec`.
- Manual `/compact` returned `status: compacting` then `status: idle`.
- A follow-up after manual compact used `shell_exec` and reported 12 checkpoint files plus the summary first line.
- The final passing TUI process closed normally with `/exit`.

## Pass Criteria

- Abort converges to idle.
- Resume or follow-up completes through real provider and real tools.
- Compact after resume does not corrupt replayed Claude Code input messages.
- Post-compact turns can still call tools.
- TUI transcript patches do not repeatedly re-render the same abort or error block.
- No orphaned `tool_use` or `tool_result`.
- No duplicate execution of completed checkpoint files after resume.

## Failure Signals

- Session stays busy after abort.
- Resume fails because provider replay contains invalid thinking payloads.
- Compact leaves the TUI idle but later tool calls fail.
- TUI repeatedly prints the same abort/error block from transcript patches.
- Resume loses already-completed checkpoint state or reruns completed checkpoint files.

## Process Record

### Run 1

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-tui-compact-abort-resume-7r1ixv`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-tui-compact-abort-resume-7r1ixv/tui-compact-abort-resume.log`
- Prompt: checkpoint recovery prompt above.
- Abort timing: after initial shell setup began.
- Resume path: `/resume`.
- Verdict: Failed.
- Failure signature: Claude returned `API Error: 400 messages.1.content.1.thinking.signature.str: Input should be a valid string`.
- Product gap found: `assistant_thinking` items with `signature: null` were valid in the demi transcript, but Claude Code JSONL replay serialized them back to Claude as a thinking block with an invalid null signature.
- Fix added: Claude Code input replay now skips unsigned assistant thinking items; signed thinking is still replayed.
- Regression test added: `requestToInputMessages skips unsigned assistant thinking for Claude replay` in `packages/provider-claude-code/src/__tests__/jsonl-output.test.ts`.

### Run 2

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-tui-compact-abort-resume-9840pS`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-tui-compact-abort-resume-9840pS/tui-compact-abort-resume.log`
- Prompt: checkpoint recovery prompt above.
- Abort timing: after `CHECKPOINT_02`.
- Resume path: `/resume`.
- Compact phases: usage-based auto compact after resumed completion, then manual `/compact`.
- Verdict: Failed UI quality gate; core runtime path passed.
- Runtime evidence: no provider schema error; resume checked existing checkpoint files, completed `CHECKPOINT_03` through `CHECKPOINT_12`, wrote `SUMMARY.md`, auto compacted, then continued with shell tools after compact.
- UI failure: the TUI repeatedly rendered the same `turn aborted` block from transcript patch replay.
- Product gap found: TUI renderer de-duped streamed text/tool/usage deltas but not stable `error` and `abort` blocks.
- Fix added: renderer now tracks seen abort/error block ids and prints each only once.
- Regression test added: `renderer.test.ts` now replays the same abort/error blocks and asserts the patch delta does not contain duplicate `turn aborted` or `agent error`.

### Run 3

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-tui-compact-abort-resume-9A65iN`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-tui-compact-abort-resume-9A65iN/tui-compact-abort-resume.log`
- Log size: `90266` bytes.
- Prompt: checkpoint recovery prompt above.
- Abort timing: after initialization and before checkpoint files were created.
- Resume path: `/resume`.
- Compact phases: one usage-based auto compact after the resumed long turn, one manual `/compact`, and one post-manual-compact follow-up.
- Verdict: Passed.

Key final-pass log evidence:

- `/abort`: line 82 `abort requested`, line 84 `turn aborted`, line 85 `status: idle`.
- `/resume`: line 88 `status: running`.
- Auto compact: line 882 `usage: in=300 out=11431 cache_read=271431 cache_write=20537`, line 883 `status: compacting`, line 884 `status: running`.
- Post-auto-compact tool use: lines 901/928 and 936/981 show `shell_exec` calls executing and completing after auto compact.
- Manual compact: line 1032 `status: compacting`, line 1033 `status: idle`.
- Post-manual-compact follow-up: line 1045 executed `echo "Checkpoints: $(ls -1 artifacts/checkpoint_*.txt | wc -l) files | SUMMARY.md first line: $(head -n 1 SUMMARY.md)"`; line 1047 printed `Checkpoints:       12 files | SUMMARY.md first line: # Long Task Recovery Experiment - Summary Report`.
- Final status: line 1059 `status: idle`, line 1061 `closed`.

Negative checks on the final passing log:

- `API Error`: absent.
- `thinking.signature`: absent.
- `agent error`: absent.
- `turn aborted`: exactly one occurrence.

Non-product note: the model tried `du -b` once on macOS and received `du: invalid option -- b`, then continued with working verification. This was model command portability, not a runtime failure.

## Failure Analysis

This acceptance test found two bugs that deterministic tests had missed:

- Real `/abort` + `/resume` can replay transcript thinking items into Claude Code JSONL. Unsigned thinking is valid as a local transcript artifact but invalid as a Claude replay thinking block, so replay must skip it.
- Transcript patch replay can resend stable abort/error blocks. The TUI must render these terminal blocks by block id rather than printing them on every patch.

Both bugs were only visible when a real TUI session interleaved abort, resume, provider replay, compact, and post-compact tool calls.

## Follow-Up Deterministic Tests

- `packages/provider-claude-code/src/__tests__/jsonl-output.test.ts`: unsigned assistant thinking is skipped during Claude replay, while signed thinking remains serializable.
- `packages/tui/src/__tests__/renderer.test.ts`: repeated transcript patches for the same abort/error block do not duplicate user-visible output.
- Existing `packages/agent/src/__tests__/session.test.ts`, `packages/agent/src/__tests__/compaction.test.ts`, and `packages/agent/src/__tests__/server.test.ts` continue to cover abort, resume, compact, transcript invariants, and AgentServer/transport event propagation.
