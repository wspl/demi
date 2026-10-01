# @demicodes/protocol

The TypeScript form of Demi's Rust contracts: the Zod schemas and types of the
conversation socket's frames, the transcript, the content and the browser's
live view, and the tables and lookups the page shares with the backend, such
as the file-type table and `MAX_PAGE_MESSAGE_BYTES`. Every file is generated
from the Rust contract crates by `bun run contracts`; the package has no
hand-written source, so a Rust contract change is a change of this package.

A value from the network is parsed with its schema, and its type comes from
the schema:

```ts
import { serverFrameSchema, type ServerFrame } from '@demicodes/protocol'

const frame: ServerFrame = serverFrameSchema.parse(JSON.parse(message))
```

Demi has no TypeScript SDK: this package and
[`@demicodes/conversation-client`](../conversation-client/README.md) are the
web app's client of the conversation socket, not a way to run Demi's agent in
another program ([The TypeScript boundary](../../docs/architecture/contracts.md#the-typescript-boundary)).

Part of [Demi](../../README.md). Apache-2.0.
