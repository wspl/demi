# Codex Steer / Queue Interleaving Acceptance

Date: 2026-06-23

Status: passed

Scope: real Codex provider, real `AgentSession`, and transcript-backed same-turn continuation at a controlled tool boundary.

## Command

```sh
DEMI_CODEX_STEER_E2E=1 DEMI_CODEX_TRANSPORT=sse bun test packages/provider-codex/src/__tests__/real-codex.e2e.test.ts --timeout 420000
```

Result:

```text
1 pass
4 skip
0 fail
11 expect() calls
```

## Scenario

The gated test uses the local official Codex auth store and a real `CodexProvider`.

1. Start an active turn in an `AgentSession`.
2. The model must call a custom `wait_gate` tool exactly once before final output.
3. While the tool is blocked, call `session.send(...)` with a queued next-turn prompt.
4. While the queued prompt is pending, call `session.steer(...)` with same-turn guidance.
5. Release the tool and wait for both the active turn and queued turn to finish.

The test asserts:

- `wait_gate` is called exactly once.
- the transcript order is first user input, `wait_gate` tool call, accepted `steer`, active answer, queued user input, queued answer;
- the active answer contains both `ACTIVE_DONE` and the steered `STEER_INCLUDED` marker;
- the queued answer contains `QUEUED_DONE`;
- the queued user input does not run before the active turn completes.

## Notes

This validates the real-provider path for transcript-backed same-turn continuation at a deterministic tool boundary. Provider-stream timing is intentionally covered by deterministic Codex transport tests because a real network stream does not give a stable hook for injecting steer before completion.
