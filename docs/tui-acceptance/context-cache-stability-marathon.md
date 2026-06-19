# Context Cache Stability Marathon Acceptance Record

| | |
|---|---|
| Date | 2026-06-19 |
| Status | Passed |
| Scope | Real TUI + real Claude Code provider + repeated stable-prefix turns |
| Primary model | `claude-haiku-4-5`, thinking off |
| TUI command | `/usr/bin/script -q <log> bun run packages/tui/src/index.ts --cwd <tmp> --model claude-haiku-4-5 --no-thinking --budget 1.00 --yield-after-ms 1000 --timeout-ms 180000` |
| Acceptance target | Verify cache usage fields survive TUI rendering, real cache hits appear on provider continuation paths, compact reduces the next request shape, and cache-backed operation resumes after compact |

## Scenario Design

Run many similar turns with stable instructions and small observable outputs. Include at least one compact point. Avoid changing tools/system prompt/model within the session.

The real run also includes a multi-tool continuation turn because short one-tool turns may not produce provider cache hit fields even when the visible conversation prefix is stable.

## Machine-Checkable Evidence

- `usage:` lines include `cache_read` and `cache_write` fields.
- Short one-tool turns are recorded exactly as the provider reports them, even when cache fields are `0`.
- A multi-tool continuation before compact reports non-zero cache usage.
- Manual `/compact` returns to idle.
- A multi-tool continuation after compact reports non-zero cache usage again.
- File state proves the model continued from the same workspace and did not lose task state across compact.
- Model-visible output does not include raw cache metadata.

## Pass Criteria

- Cache fields are preserved through TUI rendering.
- Real provider continuation can produce non-zero cache read/write usage.
- Compact does not permanently destabilize provider request prefix behavior.
- The agent continues normal turns after compact.

## Failure Signals

- Cache usage fields disappear from TUI output.
- Cache pressure grows past threshold without compact.
- After compact, every turn behaves like a cold context indefinitely.
- Cache metadata leaks into assistant-visible or user-visible content.

## Process Record

### Run 1

- Date: 2026-06-19
- Workspace: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-tui-context-cache-myL1kk`
- Log path: `/var/folders/bj/xcm3f3zx2z710fbv_jt6p3zr0000gn/T/demi-tui-context-cache-myL1kk/tui-context-cache-stability-marathon.log`
- Prompt sequence:
  - Step 1: append `CACHE_STEP_01 alpha` to `cache-trace.log`, read the file, and report the last line.
  - Step 2: append `CACHE_STEP_02 beta`, read the file, and report the last line.
  - Step 3: append `CACHE_STEP_03 gamma`, read the file, and report the last line.
  - Steps 4-9: use six independent shell tool calls, each appending one line and running `tail -n 1 cache-trace.log`.
  - `/compact`.
  - Steps 10-15: first confirm the existing last line, then use six independent shell tool calls to append six more lines and run `tail -n 1 cache-trace.log`.
- Verdict: Passed.

Cache usage sequence:

- Line 46: `usage: in=6600 out=381 cache_read=0 cache_write=0`.
- Line 86: `usage: in=7138 out=372 cache_read=0 cache_write=0`.
- Line 122: `usage: in=7744 out=490 cache_read=0 cache_write=0`.
- Line 193, multi-tool continuation before compact: `usage: in=20 out=832 cache_read=4112 cache_write=5124`.
- Line 274, multi-tool continuation after compact: `usage: in=30 out=1043 cache_read=10686 cache_write=6449`.

Compact evidence:

- Line 197: `status: compacting`.
- Line 198: `status: idle`.

Workspace evidence:

- `cache-trace.log` has 15 lines.
- Line 1 is `CACHE_STEP_01 alpha`.
- Line 9 is `CACHE_STEP_09 iota`.
- Line 15 is `CACHE_STEP_15 omicron`.

Final-pass log evidence:

- The pre-compact multi-tool turn appended `CACHE_STEP_09 iota`; line 193 then reported non-zero `cache_read` and `cache_write`.
- The post-compact turn first read line 223 `CACHE_STEP_09 iota`, then appended through `CACHE_STEP_15 omicron`.
- Line 274 reported non-zero cache usage after compact.
- Line 277 `closed`.

Negative checks on the final passing log:

- `API Error`: absent.
- `agent error`: absent.
- `turn aborted`: absent.
- unexpected `tool: shell_exec error`: absent.

## Failure Analysis

The original short-turn assumption was too weak as a provider-cache oracle: three stable, one-tool user turns reported `cache_read=0 cache_write=0`. That is still useful evidence because the TUI preserved the provider-reported fields instead of inventing cache state.

The meaningful real cache signal appeared when the same Claude Code turn performed multiple tool continuations. That path matches the long-task behavior we care about: the provider keeps a stable request prefix across tool result continuations, the TUI preserves cache usage fields, and after manual compact the next multi-tool turn again reports non-zero cache usage.

## Follow-Up Deterministic Tests

- Existing provider and AgentClient usage propagation tests cover preserving `cacheReadTokens` and `cacheWriteTokens` through internal events.
- Existing TUI renderer tests cover `usage:` line rendering.
- Existing agent context-cache tests cover stable provider request shape and compacted replay. This real TUI record covers the provider-dependent cache-hit signal that deterministic tests cannot prove.
