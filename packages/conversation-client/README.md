# @demicodes/conversation-client

The browser's client of Demi's conversation socket
([Contract crates](../../docs/architecture/contracts.md#contract-crates)): `ConversationClient`
opens a conversation, sends messages, edits, steers and aborts, and keeps the
transcript current from the backend's patches; `createWebSocketTransport`
carries its frames over a WebSocket; `applyTranscriptPatches` applies a patch
list to a transcript. Every frame the backend sends is validated against the
schemas of [`@demicodes/protocol`](../protocol/README.md) before the client
acts on it, and a frame over the backend's message limit is refused locally
as the backend would refuse it.

```ts
import { ConversationClient, createWebSocketTransport } from '@demicodes/conversation-client'

const socket = new WebSocket(`wss://demi.example/api/conversations/${id}/stream`)
const client = new ConversationClient(createWebSocketTransport(socket))
client.subscribe((event) => {
  if (event.type === 'transcript_patch') {
    render(event.blocks)
  }
})
await client.open()
await client.submit([{ type: 'text', text: 'Fix the failing test.' }])
```

Demi has no TypeScript SDK: the client drives a conversation the backend
runs; it does not run Demi's agent itself
([The TypeScript boundary](../../docs/architecture/contracts.md#the-typescript-boundary)).

Part of [Demi](../../README.md). Apache-2.0.
