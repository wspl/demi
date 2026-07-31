# @demicodes/agent

The session runtime and transport-neutral client/server protocol for Demi. It
owns a turn's lifecycle — queueing, provider streaming, tool execution, steering,
compaction, abort, resume recovery, and yield wakeups — and exposes it through an
`AgentClient` that the REPL and web UI both consume.

- `AgentServer` — owns sessions; `server.client()` returns an in-process client.
- `AgentClient` — `open` / `send` / `steer` / `resume` / `abort` / `close` /
  `subscribe` / `listConversations`.
- `AgentSession.clone()` — isolated point-in-time session copies (compaction and
  forks); see [docs/provider-session-clone.md](../../docs/provider-session-clone.md).
- Transports: in-process, stdio (`@demicodes/agent/stdio`), or WebSocket
  (`createWebSocketClientTransport` from the root entry).
- Actions accept caller-defined `metadata` that is available to harness and
  tool contexts without becoming transcript or provider content
  ([docs/action-metadata.md](../../docs/action-metadata.md)).

```ts
import { AgentServer } from '@demicodes/agent'

const server = new AgentServer({ agent, providers })
const client = server.client()
await client.open(selection, cwd, sessionId)
```

`sessionId` is caller-owned and required so a conversation is never silently
un-resumable.

See [Embed the UI](../../docs/guides/embed-the-ui.md). Part of
[Demi](../../README.md). Apache-2.0.
