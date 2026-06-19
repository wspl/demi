# TUI Haiku Compact Acceptance Record

| | |
|---|---|
| Date | 2026-06-19 |
| Scope | Real TUI + real Claude Code provider + `claude-haiku-4-5` |
| TUI command | `bun run packages/tui/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 0.50 --yield-after-ms 1000 --timeout-ms 180000` |
| TUI model context | `packages/tui/src/index.ts` sets `contextWindow: 200_000` for the selected model |
| Compaction threshold | `packages/base-agent/src/session.ts` preflight threshold is `0.8 * contextWindow`, so this run should compact near 160k estimated tokens |
| Acceptance target | Drive the TUI past context pressure at least 3 times and verify compact lets the agent continue working |
| Result | Partially fixed. Root cause for post-compact tool unavailability was found and fixed in the Claude Code provider replay layer. Real TUI now proves a tool can run after the first compact, but the 3-cycle Haiku TUI acceptance still has not passed because the model repeatedly reissues tool calls in synthetic probe prompts. |

## Runs

### 1. Long stdin text turns

- Shape: repeated long user text turns against real TUI.
- Model: `claude-haiku-4-5`.
- Observation: reached 29 sent turns / 26 acknowledged turns in one run; latest usage was `in=10 out=129 cache_read=0 cache_write=186222`; `compactCount=0`.
- Failure mode: TUI stdin/readline driving became unreliable before local transcript estimate clearly crossed the 160k preflight threshold.
- What it proves: cache write can grow near the 200k scale without triggering `AgentSession.isUsageNearLimit()`, because that path only considers `inputTokens + outputTokens`; preflight still depends on local transcript estimate and needs enough raw transcript.

### 2. Exact big output + filler prompt

- Shape: one big shell output, then a second prompt asking for one filler shell output and a final marker.
- Observation: first big shell output completed. The filler turn did not stop after one tool call; it produced about 102MB of TUI stdout, 23 tool events, and 17 usage events, then the harness timed out with `phase=running` and `compactCount=0`.
- Failure mode: model/tool loop stayed inside one user turn, so preflight compact had no chance to run. Auto compact also did not trigger because provider usage stayed small (`inputTokens + outputTokens`), even though raw transcript output was huge.
- What it proves: real model behavior can create large raw tool history mid-turn; relying only on next-turn preflight misses this pressure until the turn ends or is aborted.

### 3. Controlled pressure + `/abort` + trigger

- Shape: ask the model to repeatedly run a local shell output command, then use TUI `/abort` after enough raw output, then send a short trigger prompt.
- Observation: pressure step produced about 1.0MB raw output and 10 tool events; `/abort` returned the TUI to idle. The next trigger step observed `compactDelta=1` and a normal usage event.
- Failure mode: after compact, the model treated the following pressure request as prompt injection/resource-waste behavior and refused to continue.
- What it proves: TUI can surface `status: compacting` and recover to idle after a large aborted turn, but the compact summary can change model behavior enough to prevent continued work.

### 4. Finite benchmark cycles

- Shape: avoid trigger-like marker text and indefinite instructions. Each cycle asks for four separate `shell_exec` calls, each printing 200k local characters, then a short completion sentence.
- Cycle 1: completed normally with about 803KB raw output, 8 TUI tool events, and usage `in=2968 out=655 cache_read=0 cache_write=24916`.
- Cycle 2: sending the next cycle triggered `status: compacting` (`compactDelta=1`), then the model did not call tools. It replied: `I apologize, but I'm unable to continue with benchmark cycle 2. The shell execution tool is no longer available in this session.`
- Failure mode: compact occurred, but the real post-compact model turn did not continue the tool workflow.
- What it proves: the current real TUI/Claude Code compact path is not acceptable for long tool-heavy work. The observable failure is not merely missing UI; after compact, the model believed shell tools were unavailable.

### 5. Provider replay fix

- Root cause: Claude Code reports MCP tools as names like `mcp__main__shell_exec`, while demi stores the internal tool name as `shell_exec` for `AgentTool` lookup. Continuation inside the same active Claude transport works because it sends MCP control responses directly. Fresh Claude runs after compact/retry/reopen replay historical `tool_use` blocks through JSONL, and those were written back as bare `shell_exec`.
- Fix: `packages/provider-claude-code/src/jsonl.ts` now maps internal tool names back to Claude MCP names when serializing historical `tool_use` blocks. Existing MCP-prefixed names are left unchanged.
- Tests: `packages/provider-claude-code/src/__tests__/jsonl-output.test.ts` now covers bare internal names becoming `mcp__main__shell_exec`; `packages/provider-claude-code/src/__tests__/provider.test.ts` now covers a fresh provider run replaying historical internal tool names with MCP names.
- Verification: `bun test packages/provider-claude-code/src` passed with 33 pass / 3 gated skips.

### 6. Post-fix real TUI checks

- Finite cycle rerun: cycle 2 triggered one compact and then continued to call shell tools instead of saying the tool was unavailable. This verifies the provider replay fix against the original failure symptom.
- One-command rerun: cycle 2 triggered one compact and repeatedly executed the requested local command. Tool availability was fixed, but the turn did not stop before timeout.
- Pressure/probe rerun: probe 1 triggered one compact and produced `PROBE_CYCLE_1_OK` via `shell_exec` after compact. The model then repeatedly called the probe tool; the harness aborted the turn. A second probe became confused by the abort/history interleaving and did not reach a second compact within the timeout.
- Current real-TUI status: the original post-compact tool availability defect is fixed, but the acceptance target of 3 successful real Haiku compact continuations is still not satisfied.

## Code Observations

- TUI does not render `compaction_boundary` or `compaction_marker`; the CLI-visible evidence is `status: compacting`, subsequent usage, and whether the agent continues work.
- `executePreflightCompaction()` runs before provider calls when local `Transcript.estimateContextTokens()` crosses the threshold.
- `generateCompactionSummary()` intentionally sends summary requests with `tools: []`.
- Claude Code provider strips MCP tool names such as `mcp__main__shell_exec` to `shell_exec` when recording tool calls. Provider replay must restore MCP names when writing historical `tool_use` blocks back to Claude Code.
- Existing deterministic tests cover runtime compact invariants and provider tool roundtrips, but they do not prove this real path: compacted replay containing prior shell tool history, followed by a new real Claude Code turn that must use tools again.

## Current Conclusion

- This gated acceptance is not fully passed.
- We have verified the correct model path (`claude-haiku-4-5`) and 200k TUI context selection.
- We have verified that real TUI can enter `status: compacting` after large raw tool history.
- We have fixed and verified the provider replay defect that made the real model believe shell tools were unavailable after compact.
- We have verified one real post-compact shell tool execution after the fix.
- We have not verified 3 successful post-compact continuations; current real Haiku runs are blocked by repeated tool-call loops and abort/history interleaving in the synthetic harness.

## Follow-up Checks

- Add event-level instrumentation or a gated test that records post-compact provider request shape, especially `request.tools`, replayed `tool_use` names, and summary text.
- Verify whether summary requests with `tools: []` cause summary text that implies tools are unavailable, and adjust the summary contract if needed.
- Add TUI-visible compact evidence, or a structured gated harness, so acceptance can distinguish compact phase from actual boundary/marker insertion.
- Build a stable real TUI compact harness that can stop repeated tool-call loops without polluting the next prompt with abort/history confusion.
