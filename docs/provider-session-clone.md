# Provider and Session Clone

| | |
|---|---|
| Date | 2026-07-30 |
| Status | Implemented |
| Scope | `@demicodes/provider` `AgentProvider.clone()`, `@demicodes/agent` `AgentSession.clone()`, compaction via session clone |

## Why

Products need to fork a live conversation into an isolated session without sharing
mutable provider execution state. Examples:

- ephemeral recall / memory forks that inherit a transcript snapshot;
- compaction summaries that run on a clone (see `docs/compaction-context-cache.md`).

Two layers, each with a required `.clone()`:

1. **Provider** — independent runtime, same configuration.
2. **Session** — isolated point-in-time session copy; uses provider clone by default.

## `AgentProvider.clone()`

Required on every runtime. Breaking change: custom providers must implement it.

```ts
interface AgentProvider {
  run(request: InferenceRequest): ProviderRun
  clone(): AgentProvider
  dispose?(): Promise<void> | void
}
```

Rules:

- May share configuration dependencies (auth stores, quota observers, transport factories).
- Must not share mutable per-session execution state (CLI subprocesses, sockets,
  pending tool calls, continuation / active-run state).
- The clone is independently disposable; disposing it must not tear down the parent.

## `AgentSession.clone()`

```ts
session.clone()
session.clone({ transcript, provider, model, cwd, runtime, state, options })
```

Defaults when overrides are omitted:

| Field | Default |
|---|---|
| `provider` | `this.provider.clone()` |
| `model` | `structuredClone(this.model)` |
| `cwd` | same reference |
| `runtime` | same harness runtime |
| `transcript` | `structuredClone(this.transcript().toJSON())` |
| `state` | `structuredClone(this.state())` |
| options | parent retry policy; no store inheritance |

Override notes:

- Passing `provider` skips `this.provider.clone()` — the caller owns that runtime.
- Passing `state` transfers ownership by reference (same as the constructor).
- Parent `store` is never inherited; pass `options.store` explicitly if needed.

## Compaction

Compaction summaries run through `AgentSession.clone()` +
`send(COMPACTION_SUMMARY_INSTRUCTION)`. Details and cache-prefix guarantees live in
`docs/compaction-context-cache.md`.

## Coverage

- `packages/agent/src/__tests__/session.test.ts` — snapshot isolation, provider
  ownership / dispose, override path.
- `packages/agent/src/__tests__/compaction.test.ts` / `context-cache.test.ts` —
  compaction via clone.
- `packages/provider-claude-code/src/__tests__/provider.test.ts` — independent
  live-process state across clone.
- First-party providers implement `clone()`; `StubProvider` shares the scripted
  turn cursor across parent/clone facades for deterministic multi-request tests.
