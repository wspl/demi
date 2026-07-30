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
