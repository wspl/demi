# Compaction Context Cache

| | |
|---|---|
| Date | 2026-07-30 |
| Status | Implemented |
| Scope | `@demicodes/agent` compaction summary requests |

When a session exceeds its compaction threshold, Demi summarizes an old
transcript window and replaces that replay window with a compaction boundary.
Compaction performs that work through the normal session path:

1. `AgentSession.clone()` copies the selected transcript snapshot, model, and
   agent state;
2. the clone receives an independent provider runtime with the same provider
   configuration;
3. the clone inherits the normal harness runtime, cwd, retry policy, system
   prompt, preamble, tools, and thinking selection; and
4. compaction calls the clone's normal `send()` with one summary instruction.

Compaction has no system prompt of its own. It does not prepend, append, or
rewrite the session system prompt; the only compaction-specific prompt content
is the summary user message (`COMPACTION_SUMMARY_INSTRUCTION`).

This shape preserves the token prefix of an ordinary conversation request.
Prefix-caching providers can therefore reuse the system prompt, tools, and old
history, with divergence beginning only at the final summary instruction.
The clone also inherits the model's thinking configuration; compaction does not
special-case it.

The final instruction asks the model not to obey instructions in the history
and not to call tools. If the model nevertheless requests a tool, the clone
uses the inherited normal tool loop and its copied state. Tool-side state
changes therefore remain isolated from the parent session.

See also `docs/provider-session-clone.md`.

## Coverage

- `packages/agent/src/__tests__/compaction.test.ts` covers structured replay,
  clone-state isolation, the inherited tool loop, aborts, retries, and context
  overflow.
- `packages/agent/src/__tests__/context-cache.test.ts` asserts that replayed
  summary items are an exact prefix of the preceding ordinary turn request.
- Real-provider harness (not in `bun test`):
  `packages/agent/fixtures/compaction/` — DeepSeek Flash recall + window switch
  on the committed large-context fixture.

## Model changes and recovery

At an allowed model-switch boundary, AgentSession applies the selected provider
and model before attempting compaction. An unavailable previous provider is never
needed to summarize history for its replacement. A failed summary leaves the
selected model active and retains the history for another attempt.

An immediate switch during a summary cancels only the ephemeral clone and starts
again with the newest selection. Parent abort is separate. Summary revisions are
checked again before inserting a boundary, including after clone disposal, so an
obsolete result cannot commit. A next-turn switch waits for the next action.
Model-only updates retain a pending provider replacement rather than discarding it.

Summary requests still use the normal clone/send path. Context-length rejection
shrinks the history prefix at completed response boundaries. A preceding summary
is included with at least one new turn; shrinking must not strand the prefix
summary before the first available complete turn. Switching compaction checks
progress and a bounded pass limit; it reports inability to fit instead of claiming
success. Context sizes are estimates and provider rejections remain authoritative;
a single unsplittable oversized turn can fail explicitly without truncating it.

`model-switch-recovery.test.ts` covers unavailable previous providers, target
failure/retry, cancellation, deferred selection, pending provider ownership,
window reduction with fact retention, and impossible windows. Existing session,
compaction and cache-prefix tests cover normal execution and clone isolation.
