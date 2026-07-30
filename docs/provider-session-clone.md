# Provider and Session Clone

| | |
|---|---|
| Date | 2026-07-30 |
| Status | Implemented (Phase 1) |
| Scope | `@demicodes/provider` `AgentProvider.clone()`, `@demicodes/agent` `AgentSession.clone()` |

## Why

Products need to fork a live conversation into an isolated session without sharing
mutable provider execution state. Examples:

- ephemeral recall / memory forks that inherit a transcript snapshot;
- future compaction that runs a summary turn on a clone (Phase 2).

Two layers, each with a required `.clone()`:

1. **Provider** — independent runtime, same configuration.
2. **Session** — isolated point-in-time session copy; uses provider clone by default.

Phase 2 (compaction via session clone) depends on this contract and must not land
in the same change set.

## Phase 2 status

Implemented on `feat/compaction-via-session-clone`: compaction summaries run through
`AgentSession.clone()` + `send(COMPACTION_SUMMARY_INSTRUCTION)`. See
`docs/compaction-context-cache.md`.

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

## Coverage

- `packages/agent/src/__tests__/session.test.ts` — snapshot isolation, provider
  ownership / dispose, override path.
- `packages/provider-claude-code/src/__tests__/provider.test.ts` — independent
  live-process state across clone.
- First-party providers implement `clone()`; `StubProvider` shares the scripted
  turn cursor across parent/clone facades for deterministic multi-request tests.
