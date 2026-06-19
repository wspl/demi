# TUI Haiku Compact Acceptance Record

| | |
|---|---|
| Date | 2026-06-19 |
| Scope | Real TUI + real Claude Code provider + `claude-haiku-4-5` |
| TUI command | `bun run packages/tui/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 0.50 --yield-after-ms 1000 --timeout-ms 180000` |
| TUI model context | `packages/tui/src/index.ts` sets `contextWindow: 200_000` for the selected model |
| Compaction threshold | `packages/base-agent/src/session.ts` preflight threshold is `0.8 * contextWindow`, so this run should compact near 160k estimated tokens |
| Acceptance target | Drive the TUI past context pressure at least 3 times and verify compact lets the agent continue working |
| Result | Failed. Real TUI observed compact once in the successful pressure setup, but did not continue tool work after compact; 3 compact cycles were not reached. |

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

## Code Observations

- TUI does not render `compaction_boundary` or `compaction_marker`; the CLI-visible evidence is `status: compacting`, subsequent usage, and whether the agent continues work.
- `executePreflightCompaction()` runs before provider calls when local `Transcript.estimateContextTokens()` crosses the threshold.
- `generateCompactionSummary()` intentionally sends summary requests with `tools: []`.
- Claude Code provider strips MCP tool names such as `mcp__main__shell_exec` to `shell_exec` when recording tool calls. Later replay writes the stored `toolName` back into Claude JSONL as-is.
- Existing deterministic tests cover runtime compact invariants and provider tool roundtrips, but they do not prove this real path: compacted replay containing prior shell tool history, followed by a new real Claude Code turn that must use tools again.

## Current Conclusion

- This gated acceptance is not passed.
- We have verified the correct model path (`claude-haiku-4-5`) and 200k TUI context selection.
- We have verified that real TUI can enter `status: compacting` after large raw tool history.
- We have not verified 3 successful post-compact continuations; the strongest run failed immediately after the first compact because the model did not see or did not trust tool availability.

## Follow-up Checks

- Add event-level instrumentation or a gated test that records post-compact provider request shape, especially `request.tools`, replayed `tool_use` names, and summary text.
- Verify whether Claude Code replay requires MCP-prefixed tool names in historical `tool_use` blocks after the provider strips them for internal tool dispatch.
- Verify whether summary requests with `tools: []` cause summary text that implies tools are unavailable, and adjust the summary contract if needed.
- Add TUI-visible compact evidence, or a structured gated harness, so acceptance can distinguish compact phase from actual boundary/marker insertion.
