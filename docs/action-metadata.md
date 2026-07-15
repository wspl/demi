# Action Metadata

`@demicodes/agent` carries caller-defined metadata with one queued or active
action. Demi transports and clones the metadata but does not interpret it.

## Contract

Metadata is a readonly record:

```ts
type AgentMetadata = Readonly<Record<string, PortableJsonValue>>
```

`PortableJsonValue` covers JSON values plus `bigint` and `Uint8Array`, matching
the values preserved by every Demi agent transport.

Clients attach it to actions:

```ts
await client.send(content, { metadata: { tenantId: 'tenant-a' } })
await client.retry({ metadata: { tenantId: 'tenant-a' } })
await client.resume({ metadata: { tenantId: 'tenant-a' } })
await client.shellWrite(commandId, stdin, { metadata: { tenantId: 'tenant-a' } })
```

The session clones metadata when it accepts an action. Later caller mutation
does not change a queued or running action.

While an action runs, the same metadata is available from:

- prompt and preamble contexts;
- reference-resolution contexts;
- tool construction and invocation contexts;
- `AgentHarness.host` while resolving the Host for a shell operation;
- lifecycle events.

Absent metadata is represented as `null` in runtime contexts.

## Boundaries

Metadata is execution context, not conversation content:

- it is never appended to the transcript;
- Demi does not add it as a provider request field or transcript item; a harness
  may deliberately derive prompt text or tool selection from its metadata;
- it is not rendered in queue events;
- it is not part of the session checkpoint;
- Demi does not assign meaning to its keys or values.

A live queued send retains its own metadata when messages are reordered. Retry
and resume are new actions, so callers supply their metadata explicitly.
Steering remains part of the currently active action and does not replace that
action's metadata.

The standard shell tools resolve their Host from the current action metadata.
One agent session can therefore use different Hosts across actions. A Host must
be a stable object for one execution target so Demi can preserve that target's
shell state. Shell and command handles are accepted only when they belong to the
Host selected by the current metadata. Command-bridge calls do not carry action
metadata: `DEMI_SHELL_ID` routes a child process back to the Host that created it.

Yield wakeups inherit the scheduling tool invocation's metadata. A
metadata-bearing wakeup does not steer into a different active action; it stays
a separate hidden send so its execution context remains intact.

## Test Coverage

| Module | Coverage |
| --- | --- |
| `agent/src/__tests__/session.test.ts` | Metadata reaches prompt, reference, tool, and lifecycle contexts; caller mutation is isolated; provider input and transcript remain clean; queued sends and retries keep their action metadata. |
| `agent/src/__tests__/server.test.ts` | `AgentClient.send` metadata crosses the client/server transport and reaches the server-side harness context. |
| `agent/src/__tests__/host-routing.test.ts` | Metadata switches Hosts across actions, preserves same-Host shell state, and prevents cross-Host command control. |
| `agent/src/__tests__/stdio-transport.test.ts` | NDJSON transport preserves portable metadata values. |
| `agent/src/__tests__/websocket-transport.test.ts` | WebSocket transport preserves portable metadata values. |
