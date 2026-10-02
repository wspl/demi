# Conversation frame fixtures

The three JSON corpora are unchanged copies from
`crates/conversation-socket-protocol/tests/frames/fixtures/`. The Go tests
read them directly. They cover 19 client frames, 29 server frames (19 kinds),
28 refused client mutations and five accepted boundary mutations.

Run the TypeScript comparison from the repository root:

```sh
CGO_ENABLED=0 GOFLAGS=-mod=readonly go run ./tools/contractgen -ts \
  -ts-dir internal/framewire/testdata/generated ./internal/core ./internal/framewire
npm install --prefix internal/framewire/testdata/generated \
  --no-package-lock --no-save zod@4.5.4
node internal/framewire/testdata/compare.mjs \
  internal/framewire/testdata/generated/protocol \
  /Users/zan/Projects/demi-worktrees/gomig-ref/generated/packages/protocol/src/generated
```

Like core's comparison, this compares executable Zod schemas through their
input JSON Schemas, normalizing unordered sets. It checks the framewire-owned
export set, compares the 16 framewire schemas, runs both generators'
schemas against the shared fixtures and mutation expectations, and checks
client strictness and server tolerance. Cross-content checks intentionally
run only in Go, as they did only in Rust. The comparison takes under a second
once its local Zod dependency is installed. All generated comparison files
and the local dependency are ignored.
