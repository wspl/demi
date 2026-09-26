# Brief F4b: Zod → Go contract generation

Slot s2: work only in /home/user/demi-slots/s2 on branch go/wp/F4b-contracts.
Source /home/user/demi-slot-data/s2/env.sh before building or testing.
Read first: /home/user/demi/AGENTS.md, /home/user/demi/docs/internal/go/plan.md
(§ Contracts, § Rules for agents), docs/package-boundaries.md (§ Source
organization, § Module Layout Conventions), docs/demi-next/native-runtime.md
§ Contract generation and validation, scripts/rust-zod.ts and
scripts/generate-contracts.ts (the existing Zod → Rust generator: mirror its
approach and its validation semantics exactly).

Goal: Go bindings generated from the authoritative Zod schemas, the way Rust
bindings are generated today. No hand-written Go schema source.

Deliver:
1. scripts/go-zod.ts (shared Zod → Go generation tooling, the counterpart of
   rust-zod.ts) and scripts/generate-go-contracts.ts (writes every package),
   plus a package.json script `go:contracts`.
2. Output: internal/contract/<name>/zz_generated.go (ignored by git; see
   .gitignore) and one committed doc.go per package (package clause and a doc
   comment naming the Zod source). Packages, in priority order:
   - runnerwire: packages/runner-protocol (backend↔runner messages, managed boot, constants)
   - manifest: packages/command-loader/src/manifest/schema.ts
   - cmdservice: packages/command-protocol/src/index.ts (native wire, descriptors, edit journal, constants)
   - browser: packages/browser-protocol (index and live)
   - machineswire: packages/machines/src/{protocol,wire}.ts
   - core: packages/core
   - agentproto: packages/agent/src/protocol
3. Semantics (must equal Zod): objects → structs; unknown-key handling as the
   Zod object mode says; optional (may be absent, never null) vs nullable
   (may be null) kept distinct on decode and encode; literals, enums, ranges,
   lengths, regex patterns and refinements that rust-zod supports; discriminated
   unions → a sealed Go interface plus one struct per variant and a decoder that
   switches on the tag; bytes → []byte; dates → time.Time; exported constants →
   Go consts. Every package exposes decode functions that parse and validate at
   once (a value from outside the process is validated at entry; never return
   an unvalidated value) and encoders that refuse invalid values.
4. Encodings: JSON for all; MessagePack for runnerwire exactly as the TS codec
   (packages/runner-protocol/src/codec.ts), including the timestamp extension.
   Choose one established Go MessagePack library and use it fully.
5. Conformance tests (Go, in internal/contract/<name>/): a corpus recorded with
   the TypeScript codecs (write a small bun script under
   internal/contract/testdata-gen or scripts/, your choice, and commit the
   recorded fixtures under internal/contract/<name>/testdata/) that Go decodes
   and re-encodes byte for byte, plus rejection cases for every constraint kind.
Owned paths: scripts/go-zod.ts, scripts/generate-go-contracts.ts, the recording
script, internal/contract/**, the `go:contracts` line in package.json, and
go.mod/go.sum for the dependencies you add (list them in your report).

Rules for agents (from the plan): work only in your slot and owned paths; never
touch another slot, go/main or docs/internal (read-only). If the design does not
answer a question, stop that part and report the exact situation. Follow
AGENTS.md. Fork only MIT/BSD/Apache code. Test behavior at boundaries; never
call a real model. Commit on your WP branch; never push.
Checks before reporting: `bun run go:contracts && gofmt -l internal && go vet
./internal/contract/... && go test -race ./internal/contract/...`.
Report: what changed, files, tests and results, deviations, open questions,
design gaps.
