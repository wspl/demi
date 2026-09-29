# Browser contract comparison

Run from `go/`:

```sh
go run ./agentproto/internal/contracts -out ../packages/protocol/src/generated/contracts.ts -judge /home/user/demi-worktrees/wt-1/packages/protocol/src/generated/contracts.ts
go run ./agentproto/internal/contracts -tables-out ../packages/protocol/src/generated/tables.ts -tables-judge /home/user/demi-worktrees/wt-1/packages/protocol/src/generated/tables.ts
go run ./agentproto/internal/contracts -web-out ../packages/web/src/api/generated/web-api.ts -web-judge /home/user/demi-worktrees/wt-1/packages/web/src/api/generated/web-api.ts
```

Refresh the judge output in wt-1 with
`flock /tmp/demi-suite.lock target/debug/xtask contracts` first. The checked-in
Rust goldens come from that command. Comparisons fail on any byte difference;
the web comparison identifies every affected root and its local dependencies.
Protocol imports belong to the protocol comparison.

All 88 web roots now match exactly, including foreign documentation. The
available protocol roots match; `LiveModuleMessage`, `LiveViewerMessage` and
13 live-view constants still wait on L4. Their partial output is not ready
for browser integration.

Core and agentproto use ordinary `cmd/wiregen`; core/internal/generate and
core/wireadapter are removed. Patches use native **T fields. BrowserTab uses
its generated owner validation. Nested syntax errors use typed causes, with
duplicate object names still classified as shape errors.

Remaining declaration gaps after a826cb18:

- Foreign embedded structs: AccountDTO embeds core.AccountInfo, CatalogModel
  embeds core.ProviderModel, WorkingTreeChanges embeds runnerproto.GitChanges.
  The parser's embed function accepts only a local identifier.
- Trimmed is a named string with a normalizing decoder. Native generation casts
  the raw string directly and bypasses that decoder. An opaque struct cannot
  replace it while retaining field chars rules: those require KindString.
- Opaque schema metadata provides string/format, but not scalar length/pattern
  bounds, named versus inline schemas, or ConfiguredModels' array shape.
- Foreign aliases of containers: agentproto.Failures is exactly
  map[string]core.ProviderFailureFacts. Web declarations spell that identical
  type directly, with the same key/value checks, so native codecs work.

The small, explicitly temporary `webapi/wiredecl` bridge covers only foreign
embeddings and Trimmed's decode hook. Browser metadata beside declarations
covers the remaining schema shapes. It does not edit wiregen's parser, model
or Go emitter. These bridges remain until L5 adds the missing declarations;
all previously supported adapter transformations were removed.

L5 will migrate runnerproto.NormalURL to core.ParseURL with its boot-specific
URL restrictions. EndpointURL already uses the shared WHATWG parser.

Authorized foreign comment changes are BrowserTab (builtinproto/browser.go),
CommandLocale (commandservice/invocation.go), RunnerPlatform
(runnerproto/values.go), GitChange including Status and From, and GitChanges.Head
(runnerproto/messages.go). No behavior in these owners was changed.

URL/path oracle source is retained at
webapi/internal/endpoint/testdata/oracle.rs; the fixtures record Rust url 2.5.8
and typed-path 0.12.3 answers. It was compiled against judge rlibs with rustc,
without a second Cargo target directory.
