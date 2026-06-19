# TUI Haiku Compact Acceptance Record

| | |
|---|---|
| Date | 2026-06-19 |
| Scope | Real TUI + real Claude Code provider + `claude-haiku-4-5` |
| TUI command | `bun run packages/tui/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 0.50 --yield-after-ms 1000 --timeout-ms 180000` |
| TUI model context | `packages/tui/src/index.ts` sets `contextWindow: 200_000` for the selected model |
| Compaction threshold | `packages/base-agent/src/session.ts` preflight threshold is `0.8 * contextWindow`, so this run should compact near 160k estimated tokens |
| Acceptance target | Drive the TUI past context pressure at least 3 times and verify compact lets the agent continue working |
| Result | Passed after fixing Claude Code provider replay names, repeated MCP request-id handling, and runaway repeated `shell_exec` control. Real TUI exceeded the 200k context pressure path repeatedly, observed 4 compact phases, and continued after compact at least 3 times. |

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
- Intermediate real-TUI status: the original post-compact tool availability defect was fixed, but the 3-cycle target was still blocked by repeated tool-call loops.

### 7. Repeated MCP id and repeated command fixes

- A later real TUI rerun produced only one compact. Its log showed a new `pressure2` `shell_exec` call, followed by an old `pressure1` tool block changing to error. That pointed to tool result completion matching an earlier historical block with the same tool id, leaving the current tool call pending; pending tool calls make compaction no-op by design.
- Root cause: Claude Code MCP `tools/call` request ids are transport-level JSON-RPC ids, not durable transcript tool ids. Reusing them as `toolUseId` can collide across repeated MCP calls and corrupt `tool_use -> tool_result` pairing.
- Fix: `ClaudeCodeProvider` now assigns a unique `mcp-control-*` transcript `toolUseId` for each MCP `tools/call`, while still responding to the original MCP request id. `Transcript.completeToolCall()` now completes only the latest pending matching tool call, so completed historical blocks are not rewritten.
- Additional control fix: rapid identical `shell_exec` scripts are suppressed after repeated consecutive attempts in the same agent session, and the session can stop the current turn after such terminal tool results. This prevents one real model turn from looping forever on the same local command.
- Tests: `ClaudeCodeProvider keeps repeated MCP request ids distinct in AgentSession`, `Transcript completes the pending tool call when tool ids repeat`, `AgentSession can stop a turn after a terminal tool result`, and `shell_exec suppresses rapid repeated identical scripts for an agent session`.

### 8. Passing real TUI acceptance

- Command shape: `bun run packages/tui/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 1.00 --yield-after-ms 1000 --timeout-ms 180000`.
- Pressure shape: four finite local `shell_exec` pressure turns, each asking for one Python command that writes one marker line plus 720k repeated characters, followed by a no-tool continuation check.
- Log size: 2,887,721 bytes.
- Process result: exit code 0.

| Step | Compact delta | `shell_exec` render delta | Usage delta | Outcome |
|---|---:|---:|---:|---|
| pressure 1 | 0 | 2 | 1 | Completed without compact; seeded context pressure. |
| pressure 2 | 1 | 2 | 1 | Compact ran before the turn; shell tool executed after compact; completion reply observed. |
| pressure 3 | 1 | 2 | 1 | Second compact; shell tool executed after compact; completion reply observed. |
| pressure 4 | 1 | 2 | 1 | Third compact; shell tool executed after compact; completion reply observed. |
| continuation 5 | 1 | 0 | 1 | Fourth compact; no-tool continuation reply observed. |

Final counters: `compacting=4`, `shellExec=8`, `usage=5`, `suppressed=0`, `toolUnavailable=0`, `idle=6`.

## Code Observations

- TUI does not render `compaction_boundary` or `compaction_marker`; the CLI-visible evidence is `status: compacting`, subsequent usage, and whether the agent continues work.
- `executePreflightCompaction()` runs before provider calls when local `Transcript.estimateContextTokens()` crosses the threshold.
- `generateCompactionSummary()` intentionally sends summary requests with `tools: []`.
- Claude Code provider strips MCP tool names such as `mcp__main__shell_exec` to `shell_exec` when recording tool calls. Provider replay must restore MCP names when writing historical `tool_use` blocks back to Claude Code.
- MCP request ids are not transcript tool ids. Provider-generated transcript tool ids must stay unique even when Claude Code reuses JSON-RPC ids.
- Existing deterministic tests cover runtime compact invariants and provider tool roundtrips, but they do not prove this real path: compacted replay containing prior shell tool history, followed by a new real Claude Code turn that must use tools again.

## Current Conclusion

- This gated acceptance is passed for `claude-haiku-4-5` with the TUI 200k context configuration.
- We have verified the correct model path (`claude-haiku-4-5`) and 200k TUI context selection.
- We have verified that real TUI can enter `status: compacting` after large tool history at least 4 times in one session.
- We have fixed and verified the provider replay defect that made the real model believe shell tools were unavailable after compact.
- We have fixed and verified repeated MCP request-id handling so post-compact tool results complete the current pending tool call rather than mutating historical tool blocks.
- We have verified at least 3 successful post-compact continuations with real shell tool execution, followed by one no-tool continuation after another compact.

## Follow-up Checks

- Convert the manual harness into a gated automated TUI compact e2e when cost and runtime are acceptable.
- Add TUI-visible compact evidence if users need to inspect boundary/marker insertion directly rather than inferring it from `status: compacting`.
