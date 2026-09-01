# Demi Next: Implementation Log

Live status per roadmap milestone of `docs/demi-next.md`. Updated as the work
happens — status, pitfalls, and conclusions land here so the effort can be
resumed and reviewed at any point.

Working branch: `feat/demi-next` (all implementation happens on this single
branch, renamed from the design branch `feat/chat-gui-gateway-design`).

## M0 — Groundwork

Status: **done** (2026-08-31). All three items landed and verified with
scoped `bun test` runs; repo-wide `bun run typecheck` green.

### Pre-flight (done)

- Verified the design record's cited code facts against the repo before
  starting; all hold (claude-code hard-coded `child_process.spawn` +
  `process.env`, `createLogicalHostCwd`, host-routing tests, `authStore?`
  injection points, no existing remote-Host code anywhere).
- ~~Committed the dangling `just-bash` submodule pointer (`d9d2c82`)~~ —
  **wrong, reverted** (`d75be59`): the dirty state was the working tree
  *lagging* at an older commit, not a new change; shell needs `7ed8adb`
  (hostCwd dirfd + spawn-error classification). Lesson: compare commit
  dates/ancestry before accepting a dirty submodule pointer.

### Pitfalls hit while landing M0 (all fixed on this branch)

- **Baseline typecheck was red** (pre-existing): grok-build narrowing +
  `HeadersInit`; fixed in `da997c1`. `bun run typecheck` is green again —
  keep it green.
- **just-bash needs a local build**: the workspace links
  `packages/just-bash/packages/just-bash` whose `dist/` is gitignored; after
  a submodule update run `bun install && bun run build && bun run build:worker`
  inside `packages/just-bash` or shell/agent tests fail en masse.
- **macOS cwd dirfd anchor was broken** (pre-existing, hit all real-spawn
  shell tests on macOS): `/dev/fd/N` cannot be opened or traversed for
  directory fds (ENOTDIR/ENOENT, Bun 1.3.11); the anchor is now Linux-only,
  darwin uses logical path semantics, and a failed spawn's stdio no longer
  throws `ERR_STREAM_PREMATURE_CLOSE` through `Host` streams (`714b440`).
  Consequence for the runner protocol: dirfd-stable cwd is a Linux-only
  nicety; the proxy Host's `createLogicalHostCwd` plan is unaffected.

### Item 1 — claude-code injectable spawn + env overlay

Status: **done** (`a43a744`). Tests: `transport.test.ts` (spawn injection,
env cleanliness, stream-json round trip, local wrapper smoke),
`cli.test.ts` (overlay precedence), utils `utf8Lines`.

Decisions:

- `@demicodes/provider-claude-code` must not import `@demicodes/shell`
  (package boundary), so the injectable spawn is typed **structurally** in
  `transport.ts` as a subset of `Host.process.spawn`
  (`ClaudeSpawnParams`/`ClaudeSpawnHandle`) — a real `host.process.spawn` is
  directly assignable.
- One transport implementation over the spawn-handle shape; the local
  default wraps `child_process.spawn` into the same shape (no second code
  path).
- `buildClaudeEnv` gains a public `overlay` applied last (backend sets
  `ANTHROPIC_BASE_URL` + `CLAUDE_CODE_OAUTH_TOKEN`; overlay wins over the
  authStore-resolved token).
- With an injected spawn the env base is **empty** (never leak the backend's
  `process.env` across the wire); the spawn implementation owns merging with
  the target machine's environment. The local default keeps `process.env` as
  base, unchanged.
- Local-only `resolveSpawnCwd` (statSync fallback for virtual cwds) stays in
  the local wrapper; injected spawns receive the request cwd as-is.
- Per-session spawn routing (which runner a given session's CLI lands on) is
  deliberately **not** solved here — the option is provider-level; M5's
  backend owns routing.

### Item 2 — provider execution-requirement capability flag

Status: **done** (`a8269d0`). `Provider.requiresProcessCapableHost?: boolean`
— a flat optional flag, no grouping structure (nothing else to group yet).
claude-code declares it; enforcement (virtual target refusal with upgrade
guidance) lands with `@demicodes/host-virtual` in M2.

### Item 3 — host-switch integration test (migration primitive)

Status: **done** (`734ae4f`,
`packages/agent/src/__tests__/host-switch-migration.test.ts`).

Conclusions for M6's product implementation:

- The context-block injection point is the **harness `preamble` hook**
  (`AgentHarness.preamble(ctx)` with `ctx.metadata`) — it lands on the user
  block (`Block.preamble`) and is replayed to the model prepended to the
  user turn. No new agent mechanism is needed for switch announcements.
- Per-Host `BashEnvironment` reuse verified across an excursion: A → B → A
  returns to A's shell with cwd and files intact; B starts fresh.
- The harness `host()` hook is also called in metadata-less contexts
  (session open); a product harness must fall back to the session's current
  target there rather than requiring action metadata.

### Other conclusions from landing M0

- The root-entry boundary test (`platform-entrypoints.test.ts`) rejects
  root exports from claude-code's `./transport`; the public injectable
  spawn shape types therefore live in their own `./spawn` module
  (`2059dab`) — transport stays internal.

## M1 — Runner protocol core

Status: **done** (2026-08-31, `58bbfaf`, `02154de`). Ran before M2 per the
risk-first ordering principle (the only greenfield contract); no parallel
work.

What landed:

- `@demicodes/runner-protocol` (platform-neutral, certified by the
  entrypoints boundary test): wire messages, portable frame codec,
  backend-side `RemoteHost`, runner-side `HostRpcServer`.
- `@demicodes/runner`: `RunnerClient` (outbound WS, reconnect/backoff,
  claim handshake client, ping/pong), `RunnerState` (`runner.json` +
  `runner-token` 0600), `demi-runner` CLI entry.
- M1 acceptance test passes end-to-end over a real WebSocket: real
  cat/tee/spawn on the runner, mid-command runner death → ordinary tool
  error + session continues, auto-reconnect → next command succeeds.

Wire decisions beyond the design table (the table said the op set is fixed
when the package lands):

- `hello` carries a `protocol` version integer — the backend must be able
  to tell an incompatible runner from a broken one.
- `spawn_stdin_end` added (wire form of `closeStdin`).
- fs errors carry `{message, code}` and the proxy rethrows with `code`
  intact — errno codes (ENOENT…) drive shell behavior and must survive the
  wire.
- Trailing `undefined` fs args are trimmed so the wire carries real arity.

Pitfalls:

- **`Buffer.toJSON` broke the portable codec** (pre-existing): `toJSON`
  runs before the `JSON.stringify` replacer, so Buffers (and Dates)
  arrived as plain objects. Fixed in utils by recovering originals from
  the replacer's holder (`this[key]`). Same trick was needed for the new
  Date support.
- The platform-entrypoints test scans source *text* — even a comment
  containing the word "Buffer" fails a platform-neutral package.
- Long-running silent commands emit no `shell_output` frames while
  running, so tests can't use events to detect "command started";
  `RemoteHost.activeSpawnCount` was added as the honest diagnostic.

## M2 — Backend skeleton + virtual default

Status: **done** (2026-08-31; `115e752` host-virtual, `d414327` coding-agent
host resolver, `ef75e5c` agent session-lifetime refactor, `8f25145` backend).
M2 acceptance tests pass end-to-end over the real Web API + WS stream.

What landed:

- `@demicodes/host-virtual`: `VirtualHost` (chroot-clamped per-conversation
  namespace, symlink containment, real-root-hiding realpath, hardcoded
  per-file/total quotas with artifact exemption, spawn refusal with
  guidance) + `scopedFsBackend` (real-directory adapter). Shell contract
  gained `HostSpawnError.detail`, surfaced in spawn-error stderr — that is
  the "upgrade to a device" message mechanism.
- `@demicodes/shell/testing`: shared test helpers (`memoryHostStore`).
- coding-agent: `host` option accepts the `AgentHarness.host` resolver
  signature; reference resolution follows the resolved Host.
- **`@demicodes/agent` session-lifetime refactor (the significant find)**:
  the design told us to "verify a binding close never aborts an in-flight
  turn" — it did (binding close and takeover both disposed the session).
  Sessions now live in the server: `LiveSession` (owned by the ownership
  registry) carries the AgentSession + subagent supervisor + per-Host
  shell environments and emits frames through a swappable sink. Socket
  drop = detach only; re-open adopts the live object; takeover moves the
  attachment; only the explicit `close` frame or `AgentServer.close()`
  disposes. Consequence for clients: `AgentClient.close()` sends `close`
  and *disposes* — a browser "refresh" is just dropping the socket.
- `@demicodes/backend` on **Hono** (user selection): storage module
  (dual-dialect SQL seam, bun:sqlite WAL driver, numbered migrations for
  the full final-state schema, `DbHostStore` with single-upsert atomic
  writes, conversation index), Web API (auth stubs, conversations CRUD,
  cold transcript, model catalog), per-conversation frame-protocol WS
  with server-side session/cwd scoping and first-message default titles,
  one stable `VirtualHost` per conversation, operator provider assembly
  (`demi-backend` env-key entry).

Deviations from the design record's letter (same intent):

- Migrations are numbered tagged-template constants in one module rather
  than `.sql` files — a published bundle needs no file-tree lookup; the
  numbering and common-subset SQL discipline are unchanged.
- Idle detached sessions currently stay in memory until server close;
  an eviction policy (dispose after idle, restore from checkpoint on next
  open) is a later optimization, not v1 scope.

Pitfalls:

- macOS `/var` → `/private/var`: `scopedFsBackend` must compare realpath
  results against the *canonicalized* root.
- Hono's `upgradeWebSocket` cannot reject inside the handler factory —
  do the conversation lookup in the route before delegating the upgrade.

## M3 — Storage final shape

Status: **done** (2026-09-01; `ab8c276` shell/testing entrypoint, `0e54834`
agent persistence contract, `2176685` acceptance tests, `316cd35` backend
two-plane storage). Repo-wide typecheck green; scoped tests green
(agent 248, backend 10, plus shell/runner/host-virtual sweeps).

What landed:

- **`@demicodes/agent` persistence contract**: `AgentSessionStore` is now
  `save(update)` + `load()`; a save carries only the block rows mutated
  since the last tick (`changedBlocks` + `blockCount`) plus the small state
  snapshot. Dirty tracking is fed from the existing transcript patch
  stream: position-shifting ops (add/remove/replace-all) dirty a floor
  index to the end, point ops (replace_block/append_text) dirty single
  rows; a failed save merges its marks back so the next tick retries.
  `fromCheckpoint` starts clean (its blocks came from the store).
- **`hostAgentSessionStore`**: the HostStore realization (one entry per
  block under `<prefix>/blocks/<index>.json` + `state.json`), used by the
  agent server default path and subagent children.
- **Media externalization**: `BlobStore` interface + externalize/rehydrate
  in the persistence layer — inline binary/base64 media sources become
  content-addressed `ref` sources at rest and rehydrate on load; a missing
  blob degrades to a text placeholder instead of failing the load. The
  in-memory transcript and providers always see inline bytes.
- **`@demicodes/backend` two-plane storage**: `control.sqlite` (migrations
  without host_store) behind the `ControlService` interface
  (`LocalControlService`; async domain methods, RPC-realizable at N>1) +
  `conversations/<id>.sqlite` per conversation (blocks / session /
  host_store tables) via `ConversationStores`, with `DirBlobStore`
  (`blobs/<sha256>`, temp+rename writes). `AgentServer` gained
  `sessionStore` / `blobs` injection options; the backend injects the
  per-conversation store, so the product never uses the Host-store path.
- Acceptance tests per the verification table: incremental rows across
  turns (earlier rows never rewritten), store round trips, stale-row
  deletion, media ref round trip + missing-blob degradation,
  per-conversation host_store isolation, and a full backend restart
  restoring cold + live transcript from the conversation database.

Decisions:

- Dirty marking is deliberately conservative (a mid-transcript insert
  dirties the suffix): patch indices recorded after a shift stay accurate
  below the floor and are swallowed by it above — correctness without
  replaying index arithmetic. Appends only dirty the streamed tail, which
  is the hot path that matters.
- Media externalization lives in the persistence layer, not ingestion:
  in-memory blocks keep bytes (unchanged memory profile, zero provider
  changes, no core content-type unions touched); only the at-rest
  representation carries refs.
- `ControlService` methods are async from day one so the Remote (RPC)
  implementation is a drop-in at M10.

Pitfalls:

- `bun test packages/agent` also globs `packages/agent-eval` fixtures,
  which contain an intentionally failing test — scope runs to
  `packages/agent/src`.
- Root tsconfig resolves workspace subpath imports through `paths`, so a
  new `/testing`-style entrypoint needs an explicit mapping there in
  addition to package.json `exports` and the tsdown entry list.

## Database design review (2026-09-01)

Status: **concluded** — design record updated (`demi-next.md` § Database,
Backend topology, package-changes item 4, storage-pluggability audit,
roadmap M3/M9/M10 + verification rows; `session-storage-and-naming.md`
journal role). **No implementation yet**; M3 scheduling to be confirmed
separately.

Final design (see `demi-next.md` for the full record + diagrams):

1. **SQLite only, both topologies** — the dual-dialect (SQLite/Postgres)
   plan is retired.
2. **Attachment bytes never enter any database** (implemented early:
   `b731eea`); transcript media leaves via `source.ref` + blob store.
3. **The journal is required design, not an optimization**: transcript
   persisted as one row per block in `conversations/<id>.sqlite`;
   whole-checkpoint rewrites are ruled out as a final write path.
4. **Homogeneous shape**: N=1 and N>1 use the identical data directory
   (`control.sqlite` + per-conversation files + blob store); scaling
   changes process placement only — no storage evolution layers.
5. **N>1 topology**: symmetric workers (each owns its users'
   conversation files and serves the full public API) + one internal
   `demi-controld` (sole owner of `control.sqlite`, `ControlService`
   domain RPC over Hono, no public listener). Litestream replicates every
   `*.sqlite` to S3; failover and user migration are restore-based.

Review path and verdicts (kept so the reasoning survives):

- Self-built multi-node SQLite sharding with S3 leases/epoch fencing —
  **rejected** (invented machinery). The *sharding shape* itself was later
  validated by the Tailscale precedent; only the invented parts stay dead.
- Expensify Bedrock — healthy-looking but ~no external adoption in ten
  years; **rejected**.
- sqld / libsql-server — server tree frozen upstream since 2025-12;
  **rejected**. Turso Cloud / Cloudflare D1 validate the "fleet of small
  SQLite databases as a service" pattern but are proprietary.
- rqlite — active and credible, but no interactive transactions
  (batch-only) and a Raft write ceiling; **archived** with Postgres as the
  fallbacks if multi-replica strong consistency ever becomes a real
  requirement.
- Postgres as the N>1 dialect — **superseded**: once the review removed
  the shared-database role (worker-local conversation files + a dedicated
  control-plane service), the dual-dialect cost bought nothing.
- Production precedents the final design stands on (all verified during
  the review): Bluesky PDS (per-account SQLite + service-level SQLite,
  WAL, LRU handle cache), Tailscale control plane (per-shard SQLite + one
  exclusive process, tenant migration between shards), Litestream
  (checkpoint-takeover WAL replication to S3; v0.5 LTX + tiered
  compaction; `dir`+`watch` discovery for dynamic per-tenant files),
  database-per-service (microservices.io / AWS / Microsoft guidance),
  dedicated low-write metadata services (HDFS NameNode, TiDB PD, Kafka
  controller), HTTP services fronting SQLite (Grafana, Gitea, Headscale).

Naming decided: interface `ControlService` (impls `LocalControlService` /
`RemoteControlService`), process `demi-controld`, database
`control.sqlite`. "Control plane" stays a layer term in prose; `*Store`
stays reserved for storage backends.

Pitfalls (of the review itself):

- Whole-checkpoint throttled rewrites looked acceptable ("~100s of KB")
  until multiplied by active-session count and Litestream/WAL amplification
  — write-path shape, not size, was the real issue.
- Two plausible-sounding intermediate designs died on the "does anyone
  actually run this?" test (S3-lease sharding, Bedrock). The test that
  settled every argument was demanding a named production system per
  design element.
