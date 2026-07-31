# Embed the UI

Every front-end in Demi — the terminal REPL and the browser UI — talks to the
same `AgentClient` protocol. To embed Demi in your own app you consume an
`AgentClient`: subscribe to its events to render, and call its methods to drive a
turn. You never couple to the session internals.

## Get a client

An `AgentClient` wraps a transport. Pick the one that matches where the session runs:

```ts
import { AgentClient, AgentServer, createWebSocketClientTransport } from '@demicodes/agent'
import { createStdioClientTransport } from '@demicodes/agent/stdio'

// 1. In-process — the server runs in the same process (tests, a local app):
const server = new AgentServer({ agent, providers })
const client = server.client()

// 2. Over stdio — the session runs in a child process (Node-only subpath):
const client = new AgentClient(createStdioClientTransport(child.stdout, child.stdin))

// 3. Over WebSocket — the session runs on a server:
const client = new AgentClient(createWebSocketClientTransport(socket))
```

All three expose the identical surface, so your UI code is transport-agnostic.

## Render from events

`subscribe(listener)` returns an unsubscribe function. The listener receives
protocol events; the two that carry the transcript are `transcript_reset`
(full block list replacing client state) and `transcript_patch` (incremental
updates). Render the `Block`s — the same blocks the REPL and web UI render:

```ts
const unsubscribe = client.subscribe((event) => {
  switch (event.type) {
    case 'transcript_reset':
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
shared, platform-neutral helpers (e.g. `trimToolSummary`) you can reuse. Tool blocks
carry bounded UI data on `block.view` (for shell tools, a `ShellToolView` with
`chunks`), not unbounded command status dumps.

## Drive a turn

```ts
const sessionId = crypto.randomUUID()
await client.open(selection, cwd, sessionId)       // ProviderSelection + cwd + caller-owned session id
await client.send([{ type: 'text', text: 'hi' }])  // queue/run a user turn
await client.steer([{ type: 'text', text: 'also…' }])  // interject into a running turn
const result = await client.abort()                // interrupt
await client.close()                               // tear down
```

`sessionId` is stable and caller-owned: reconnecting with the same id resumes that
conversation. Build the `ProviderSelection` from a provider's catalog with
`modelSelectionFromCatalog` (see [add-a-provider.md](add-a-provider.md)) rather than
hand-assembling a `Model`.

## Reference UIs

- `@demicodes/repl` — a terminal renderer over the in-process/stdio client.
- `@demicodes/web-ui` — a Vue UI that consumes an **injected** `AgentClient`, so the same
  components work against any transport. It is the template for a custom browser UI.
- `@demicodes/web` — the Demi web product (Vite browser app + embedded Node/Bun backend)
  that wires `@demicodes/web-ui` to a local `AgentServer`.
