# Conversation frame fixtures

The three JSON corpora are unchanged copies from
`crates/conversation-socket-protocol/tests/frames/fixtures/`. The Go tests
read them directly. They cover 19 client frames, 29 server frames (19 kinds),
28 refused client mutations and five accepted boundary mutations.

The generated TypeScript is compared with the Rust reference by the one
transitional checker, from the repository root, after `bun run contracts`:

```sh
bun scripts/gomig/compare-ts.mjs
```

For framewire it compares executable Zod schemas through their
input JSON Schemas, normalizing unordered sets. It checks the framewire-owned
export set, compares the 16 framewire schemas, runs both generators'
schemas against the shared fixtures and mutation expectations, and checks
client strictness and server tolerance. Cross-content checks intentionally
run only in Go, as they did only in Rust. The comparison takes under a second.
