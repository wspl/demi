# Conversation frame fixtures

The three JSON corpora are the conversation socket's wire frames: 19 client
frames, 29 server frames (19 kinds), 28 refused client mutations and five
accepted boundary mutations. They are recorded once; nothing regenerates them.

The Go tests decode and re-encode every frame and apply every mutation. The
web app's contract test (`packages/protocol/src/__tests__/contracts.test.ts`)
reads the same files and checks that the generated Zod schemas accept and
refuse the same frames, with client frames strict and server frames tolerant.
Rules that look across a frame's content run only in Go
([Contracts](../../../docs/architecture/contracts.md#generated-typescript)).
