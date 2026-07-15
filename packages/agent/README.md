# @demicodes/agent

The session runtime and transport-neutral client/server protocol for Demi. It
owns a turn's lifecycle — queueing, provider streaming, tool execution, steering,
compaction, abort, and yield wakeups — and exposes it through an `AgentClient`
that the REPL and web UI both consume.

- `AgentServer` — owns sessions; `server.client()` returns an in-process client.
- `AgentClient` — `open` / `send` / `steer` / `abort` / `close` / `subscribe`.
- Transports: in-process, stdio (`@demicodes/agent/stdio`), or WebSocket.
- Actions accept caller-defined `metadata` that is available to harness and
  tool contexts without becoming transcript or provider content.

```ts
import { AgentServer } from '@demicodes/agent'

const server = new AgentServer({ agent, providers })
const client = server.client()
```

See [Embed the UI](../../docs/guides/embed-the-ui.md). Part of
[Demi](../../README.md). Apache-2.0.
