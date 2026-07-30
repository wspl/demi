# Compaction Summary Styles

| | |
|---|---|
| Date | 2026-07-30 |
| Status | Design (implemented) |
| Scope | `@demicodes/agent` compaction summary request construction (`compaction-support.ts`, `compaction-controller.ts`) |

When a session's history grows past the compaction threshold, demi summarizes the oldest
transcript blocks and splices in a compaction boundary. This document records how the
summary *request* is shaped, and why.

## 1. The two styles

`AgentSessionOptions.compaction.summaryStyle` selects how the to-compact history is
presented to the model:

- **`'replay'` (default)** — the summary request is structurally identical to a normal
  turn: the session's real system prompt, the real structured `InferenceItem`s of the
  compacted window, and the session's real tool list, with the summary instruction
  appended as the final `user_message`. Built by `buildReplaySummaryRequest`.
- **`'reference'`** — the history is flattened by `renderItemsForSummary` into inert,
  delimited text inside a single `user_message` with a fixed system prompt
  ("Summarize the previous conversation…") and no tools. Built by
  `buildCompactionSummaryRequest`.

Both styles keep `thinking: null` (a generation parameter; it does not perturb prefix
tokens), and the summary string itself never enters the transcript — only the boundary
and marker blocks are written.

## 2. Cache rationale

Provider prefix caches (e.g. DeepSeek's disk cache) key on the exact request prefix. The
reference style shares *no* prefix with the session's own requests — different system
prompt, re-rendered text, no tools — so every compaction summary is a full-cache-miss.
The replay style diverges from the previous turn's request exactly at the appended
instruction, which is the theoretical optimum: the whole history prefix (system prompt,
tools, replayed items) hits the cache.

## 3. Injection trade-off

Replaying history as a real conversation means instructions buried in it (e.g. "only
reply X") are in a position the model may treat as live. The instruction appended last
frames the history as reference material that must never be obeyed, which mitigates but
does not eliminate the risk. Two guards apply:

- The instruction forbids tool calls ("Output only the summary. Do not call tools."), and
  if the summary stream still yields a tool call, the attempt is discarded and the pass
  falls back to the reference-style request (emitted as `compaction_summary_fallback`).
- Agents whose history carries **untrusted content** (tool output from the network,
  user-supplied files) can opt into `summaryStyle: 'reference'`, which keeps the history
  as inert reference material at the cost of prefix-cache reuse.

## 4. Tests

- `packages/agent/src/__tests__/compaction.test.ts` — both styles, the tool-call
  fallback, retry/abort/overflow behavior; summary requests are discriminated by the
  trailing instruction (`isSummaryRequest`), since the replay style shares the session
  system prompt.
- `packages/agent/src/__tests__/context-cache.test.ts` — the replay-style summary
  request shares the previous turn request's prefix (identical system prompt and tools,
  replayed items as a strict prefix), the property prefix caches rely on.
