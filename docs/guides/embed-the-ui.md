# Embed the UI

Every front-end in Demi — the terminal REPL and the browser UI — talks to the
same `AgentClient` protocol. To embed Demi in your own app you consume an
`AgentClient`: subscribe to its events to render, and call its methods to drive a
turn. You never couple to the session internals.

## Get a client

An `AgentClient` wraps a transport. Pick the one that matches where the session runs:

```ts
import { AgentClient, AgentServer, createStdioClientTransport } from '@demicodes/agent'

// 1. In-process — the server runs in the same process (tests, a local app):
const server = new AgentServer({ agent, providers })
const client = server.client()

// 2. Over stdio — the session runs in a child process:
const client = new AgentClient(createStdioClientTransport(child.stdout, child.stdin))

// 3. Over WebSocket — the session runs on a server (see the websocket transport).
```

All three expose the identical surface, so your UI code is transport-agnostic.

## Render from events

`subscribe(listener)` returns an unsubscribe function. The listener receives
protocol events; the two that carry the transcript are `transcript_snapshot`
(the full block list) and `transcript_patch` (an updated block list). Render the
`Block`s — the same blocks the REPL and web UI render:

```ts
const unsubscribe = client.subscribe((event) => {
  switch (event.type) {
    case 'transcript_snapshot':
    case 'transcript_patch':
      render(event.blocks)             // Block[] -> your view
      break
    case 'phase':
      setBusy(event.phase === 'running')
      break
    case 'shell_output':               // streaming shell output for a running command
    case 'tool_progress':              // long-running tool progress
      // optional: live-update the matching block
      break
  }
})
```

Block-to-view rules (titles, output truncation, stdout/stderr interleaving, the
`shell_exec` / `yield` blocks) are specified in
[docs/tool-rendering-spec.md](../tool-rendering-spec.md). `@demicodes/web-ui` ships
shared, platform-neutral helpers (e.g. `trimToolSummary`) you can reuse.

## Drive a turn

```ts
await client.open(selection, cwd)              // start a session (ProviderSelection + cwd)
await client.send([{ type: 'text', text: 'hi' }])  // queue/run a user turn
await client.steer([{ type: 'text', text: 'also…' }])  // interject into a running turn
const result = await client.abort()            // interrupt
await client.close()                           // tear down
```

Build the `ProviderSelection` from a provider's catalog with
`modelSelectionFromCatalog` (see [add-a-provider.md](add-a-provider.md)) rather than
hand-assembling a `Model`.

## Reference UIs

- `@demicodes/repl` — a terminal renderer over the in-process/stdio client.
- `@demicodes/web-ui` — a Vue UI that consumes an **injected** `AgentClient`, so the same
  components work against any transport. It is the template for a custom browser UI.
