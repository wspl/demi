# Demi Next: Implementation Log

Live status per roadmap milestone of `docs/demi-next/roadmap.md`. Updated as the work
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

## M4 — Runner productized

Status: **done** (2026-09-01; `f758090` control-plane device/workspace
records, `012d41b` runner management module, `e9ae6c9` remote-Host
resolution + acceptance, `9d0499d` CLI output polish). Repo-wide typecheck
green; backend/runner/runner-protocol suites 24 tests green.

What landed:

- **Pairing spec finalized before implementation** (`0bfdaf5`, `cee9feb`):
  claim codes are 128-bit random Crockford base32, single-use, expiring,
  claim endpoint rate-limited per user; the end-to-end flow diagram lives
  in `runner.md`.
- **`ControlService` device/workspace domain**: device rows (token stored
  as SHA-256 hash only), workspace rows, and the conversation→workspace
  pointer.
- **`backend/src/runner/` module**: `claim-codes.ts` (code generation +
  Crockford normalization with confusable mapping, device-token
  mint/hash) and `registry.ts` — pending claims keyed by normalized code
  (in memory only; expiry rotates a fresh code over the waiting socket),
  one authenticated socket per device (a newer connection replaces the
  older), backend-driven ping/pong liveness, and stable per-execution-
  target `RemoteHost`s attached/detached as the device socket comes and
  goes. Malformed frames close the socket (boundary #3 discipline).
- **HTTP surface**: `WS /api/runner` (thin adapter; all protocol logic in
  the registry), `GET /api/devices` (online = socket state),
  `POST /api/devices/claim` (404 invalid/expired, 429 rate-limited),
  `DELETE /api/devices/:id` (revoke drops the live socket and refuses the
  reconnect), `GET/POST /api/devices/:id/fs` (directory browse/create over
  ordinary Host RPC; 409 while offline).
- **Host resolution**: the harness resolves a conversation's execution
  target server-side — workspace pointer ⇒ the device's stable
  `RemoteHost` (offline ⇒ tool errors until reattach), none ⇒ virtual.
  The Host's store is the conversation's own host_store scope, same as
  virtual.
- **Runner client**: `--backend https://host` now derives
  `wss://host/api/runner` (explicit paths kept); the CLI prints the
  pairing code in the documented two-line form.
- Acceptance per the verification table: full pairing lifecycle (messy
  code input normalized, single-use, restart-with-token, revoke →
  rejected), code expiry rotation, rate limiting, malformed-frame and
  bad-token rejection, and the M4 session test — real commands and file
  edits on the claimed device through the product backend, runner death
  mid-`sleep` surfacing as a tool error with the session intact, a fresh
  runner process resuming service on the same session.

Decisions:

- Claim codes over UUIDs (user call): equal-entropy either way, but
  Crockford base32 is shorter and confusable-free; the encode/normalize
  pair is ~20 lines.
- Workspace *creation* stays M6 (workspaces CRUD): M4 adds only the
  storage primitives host resolution needs; the acceptance test writes the
  workspace row via the control plane directly.
- Fs results and spawn streams fan out to every `RemoteHost` of the device
  and each host claims its own call ids — no central id routing table to
  keep in sync.
- The registry caches each device's last-known `HostIdentity` so a Host
  can be constructed while its runner is offline (identity is read
  synchronously at shell creation).

Pitfalls:

- Backend-side "wait until the remote command started" cannot watch
  streaming shell events (they are not guaranteed mid-command); the
  acceptance test probes a filesystem marker on the runner instead.
- `waitFor` takes a sync predicate — polling an HTTP endpoint needs a
  hand-rolled delay loop.

## M5 — LLM module + credential vault + metering + Claude Code

Status: **done** (2026-09-01). Step 1 (BYOK + metering): `afa2484` agent
ProviderResolver seam, `30f8e65` vault module, `fd4d6c1` LLM assembly +
metering + HTTP surface. Step 2 (subscriptions + Claude Code): `f2710c3`
subscription login flows, `d1f15f3` design correction (below), `8b674f1`
claude-code chain. Repo-wide typecheck green; backend 27 + runner/
runner-protocol/provider-claude-code sweeps green, chain e2e passes in ~3 s
with the local `claude` binary.

Step 1 — what landed:

- **`@demicodes/agent` seam**: `AgentServerOptions.providers` accepts a
  `ProviderResolver` (`(providerId, {agentSessionId}) → Provider | null`)
  besides the static list; the binding resolves at every runtime
  construction (open, set_provider), so products assemble providers
  per-connection with session context. Static lists normalize to a
  resolver internally — one lookup path.
- **`backend/src/vault/`**: `instance-secret` (32 random bytes, 0600,
  created on first start; corrupt file is a loud error), AES-256-GCM
  `encryptJson`/`decryptJson` (`v1:iv:tag:ct`), and `ConnectionVault` —
  typed `ApiKeyConnectionConfig` plaintext exists only in process memory,
  rows carry ciphertext (`connections.type` = provider type for listing).
- **`backend/src/llm/`**: `ProviderAssembly` (connectionId = providerId;
  base provider per connection built by registered type factories —
  builtins anthropic/openai/google over the existing `create*Provider`
  factories with `id`/`displayName`/`apiKey`/`baseUrl`; cache invalidated
  on delete — connections are immutable rows), the aggregated catalog
  (live `listModels` stamped with the connectionId; compatible-endpoint
  `modelIds` become minimal entries with null capabilities), one-cheap-
  request `testConnection`, and the metering wrap (`meterProvider`):
  usage observed firsthand from `response` events — one ledger row per
  provider request — plus a `beforeRequest` enforcement hook.
- **`backend/src/usage/`**: `ProviderRateLimiter` — hardcoded generous
  per-user ceiling (120 provider requests/min, virtual-quota style),
  refusal surfaces as a `rate_limited` provider error in the turn.
- **HTTP**: `GET/POST /api/connections` (zod-validated; responses never
  carry key material), `DELETE /:id`, `POST /:id/test`,
  `GET /api/usage` (query-time aggregation, `connection × model`),
  `/api/models` rebuilt over the assembly. `BackendOptions.providers` is
  gone — `main.ts` starts credential-free and connections arrive via the
  API; tests register a `stub` provider type through
  `BackendOptions.providerTypes` and create connections over HTTP.

Step 1 decisions:

- Metering wraps the provider (not the HTTP layer): `TokenUsage` comes
  from the provider's own `response` events, which is exactly the
  "observed firsthand" requirement and the per-request granularity the
  ledger wants.
- The ledger row's `userId` comes from the conversation record at resolve
  time (multi-user-shaped now, correct when M7 lands).
- Rate-limit numbers are hardcoded like the virtual-fs quotas; no quota
  configuration surface exists by design.

Step 2 — what landed:

- **Subscription connections**: config `{kind:'subscription', provider}`;
  the OAuth material lives in the provider's OWN credential pool under the
  connection's vault directory (`dataDir/vault/<connectionId>/`) — login,
  storage format, and refresh are entirely the provider package's
  machinery; the backend only names the directory. `SubscriptionLoginFlows`
  drives `provider.credentials.beginLogin` on a throwaway pool
  (`vault/pending-<id>`), the web UI polls
  `POST/GET /api/connections/subscription-login`, and on completion the
  pool directory is renamed to the new connection's vault dir. Deleting a
  connection removes its pool.
- **Claude Code on the runner**: the claude-code type factory receives the
  session's execution-target spawn (`SessionProviderContext`, built by the
  resolver for `requiresProcessCapableHost` providers); the provider
  runtime resolves the vault token from its auth store and injects
  `CLAUDE_CODE_OAUTH_TOKEN` into the CLI env itself. The CLI talks its
  native Anthropic wire directly upstream. Chain e2e (skipped without a
  `claude` binary): real CLI on a claimed runner, mock upstream via the
  provider's public env overlay — asserts the vault token reached the
  upstream, the answer streamed back to the browser transcript, usage
  landed in the ledger, and the CLI's config home stayed inside the
  workspace artifacts dir.
- **Supporting fixes** (`8b674f1`): workspace-bound conversations open in
  their workspace path (the scoped transport had hardwired the virtual
  `/workspace` constant — the stream route now resolves the cwd from the
  workspace row); the runner fills `PATH`/`HOME` from the device when a
  spawn request names none (binary resolution is a device fact — the
  transport's remote env is deliberately built from an empty base, since
  the backend's own env is meaningless on a device); injected-spawn CLI
  runs pin `CLAUDE_CONFIG_DIR` inside the workspace artifacts dir so the
  CLI consumes zero device-local settings/hooks.

Design correction (review, 2026-09-01): the milestone was originally
implemented with an Anthropic passthrough — the CLI's traffic pointed back
at the backend (`ANTHROPIC_BASE_URL` + backend-minted token, swapped for
the vault token server-side). Review rejected it: (1) datacenter-IP
aggregation of many users' subscription OAuth is exactly the traffic shape
Anthropic bans — the ban lands on the user's account; (2) the CLI's
base-url/token behavior is not a contract (version-dependent, evidenced by
hangs); (3) the "all model traffic through the backend" invariant has no
value here — the transport already runs on the user's device and burns the
user's own subscription, so there is no credential-exposure or metering
trust gap to close; (4) the passthrough required the backend to reach into
the provider's credential internals, violating the ownership boundary.
Final shape: credentials stored server-side, injected into the CLI process
env at spawn by the provider itself, traffic direct to Anthropic
(byte-identical to a user running the official CLI); metering reads the
usage the provider reports. `d1f15f3` removed the passthrough.

Step 2 pitfalls:

- The transport's injected-spawn branch builds env from an empty base by
  design — anything device-shaped (`PATH`, `HOME`) must come from the
  device. Without the runner-side fallback the CLI spawn dies unresolved
  and the transport waits on stdout forever.
- Local CLI probes inherit the developer's own Claude Code session env
  (`CLAUDECODE`, auth vars), which changes CLI behavior — clean the env
  (`env -i`-style) before concluding anything about CLI wire behavior.
- The CLI runs device `SessionStart` hooks and settings unless
  `CLAUDE_CONFIG_DIR` is pinned — a managed device's CLI must be isolated
  or it consumes (and can be broken by) the device owner's config.

## Agent restructure & boundary schemas (2026-09-01)

Status: **done** (design record: `docs/agent-restructure-and-schemas.md`).
All six execution steps landed as separate green commits; repo-wide
typecheck clean, 543 tests across the affected packages passing.

What landed:

- Guard dedup: the two private `errorCode` clones in `@demicodes/agent`
  replaced by the `@demicodes/utils` guard.
- `@demicodes/agent` directory layout per the design record (`session/`,
  `transcript/`, `store/`, `protocol/`, `server/`, `subagent/`,
  `client/`); compaction halves merged, media externalization split from
  the session store; `./stdio` entry registrations moved with the file.
- `server/` split: facade / binding (frame dispatch) / `open-session.ts`
  (assembly pipeline, closure captures now explicit parameters) /
  live-session / ownership / summaries.
- `subagent/` split: supervisor lifecycle vs the declarative command tree
  behind a generic `SubagentCommandOps` seam (Job handle opaque, no
  internal types leak through the root export).
- Boundary schemas, one commit each: tool-progress views (zod replaces the
  40-line isRecord chain), `ClientFrame` (schemas.ts is the single source
  of truth, frames derive via `z.infer`, ingress rejects malformed frames
  with `invalid_frame`), HTTP PATCH body (400 `invalid_body`), runner
  protocol (both directions declared in `runner-protocol/schemas.ts`,
  direction-aware decode validates structurally; the in-band fs-op guard
  became redundant and was removed).
- Negative-path tests per boundary (malformed frame, malformed body,
  malformed protocol frames including a bogus fs op).

Pitfalls:

- zod's `discriminatedUnion` refuses two branches sharing a `type` value
  (the fs_result ok/error pair) — a plain `z.union` handles that message
  set with identical semantics.
- `z.instanceof(Uint8Array)` infers `Uint8Array<ArrayBuffer>`, which
  rejects ordinary `ArrayBufferLike` views at compile time; a
  `z.custom<Uint8Array>` instance check keeps the plain type.
- Shared types owned by other packages (core content blocks, provider
  selections, shell identities) keep their hand-written declarations;
  their validators are annotated `z.ZodType<T>` so schema drift fails the
  build instead of silently diverging — only types whose boundary owns
  them (frames, protocol messages) flip to schema-derived.

## Database design review (2026-09-01)

Status: **concluded** — design record updated (`storage.md`,
Backend topology, package-changes item 4, storage-pluggability audit,
roadmap M3/M9/M10 + verification rows; `session-storage-and-naming.md`
journal role). **No implementation yet**; M3 scheduling to be confirmed
separately.

Final design (see `storage.md` for the full record + diagrams):

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

## Host design round: user/managed hosts, `demi host`, lifecycle (2026-09-01)

Design-only round (no code); the outcome is the "Execution targets: user
hosts and managed hosts" section in `sessions-and-targets.md` and `managed-hosts.md`, the M6 rewrite,
the new M7 (managed hosts), and the M8–M11 renumbering.

Origin: the phrase "docker based host" existed in the design only as a
hosting variant with no usage scenario. This round gave it one and renamed
the concepts: **user host** (paired device) and **managed host**
(backend-provisioned container), serving two scenarios — session upgrade
(workspace-less conversation binds a managed host directly,
`conversations.hostDeviceId`) and Cloud workspace (one managed host per
project behind the existing workspace pipeline).

Decisions, in review order:

- **Binding model**: three-branch host resolution (workspace →
  session-bound managed host → virtual), `workspaceId`/`hostDeviceId`
  mutually exclusive; `devices.kind: 'user' | 'managed'`; managed devices
  never listed as user assets.
- **Command layering**: `demi` was found to be the file-editing command
  (read/create/edit/patch) with `todo` as a sibling — a proposed top-level
  `host` command was considered, but the user confirmed `demi host` under
  the umbrella; the backend contributes the `host` group into the
  coding-agent-owned `demi` command.
- **"upgrade" semantics rejected** in favor of target-carrying
  `demi host switch managed` (reserves `switch <device>` for future
  user-host migration).
- **`origin` rejected** as the name for the departed host (git collision;
  the concept is temporal) — `prev` chosen.
- **`ls`/`cp` migration subcommands rejected**: filtering/globbing/
  scripting belong to the shells on both ends; the only primitive is
  `demi host prev shell` as a byte-faithful ssh analog, with a portable
  create-only `tar` for virtual prevs. `demi read`'s pipe-into-ffmpeg
  contract is the plumbing precedent.
- **`release` kept over destroy/dispose**: it is the only verb true for
  all three prev types (virtual no-op / managed hibernate / user-host
  grant revocation).
- **Security baseline trimmed on review**: an earlier per-deployment
  tiering (runc for self-host, gVisor for multi-tenant) was rejected —
  uniform baseline everywhere; cap-drop/seccomp/no-new-privileges and
  split network rules were cut as redundant under gVisor; final list is
  the six items in the design.
- **Lifecycle checked against production practice** before acceptance:
  idle snapshot-and-destroy + only-home-survives is Gitpod's model
  (Codespaces/Coder as volume variants); adopted their edge cases —
  running processes count as activity, snapshot failure never destroys,
  snapshot size cap surfaces, wake idempotent per owner. The prev slot is
  the one invented piece and was deliberately kept minimal.
- The old M6 out-of-virtual tmp-dump (`/tmp/demi-migration-<id>/`) is
  superseded by the prev pipe — the agent pulls what it needs itself.

Explicitly not designed (deferred by decision, not omission): user-host
migration implementation (`switch <device>` + web confirmation flow),
managed→virtual downgrade entrance, finer security items.

### Provisioning review: Docker Engine API dropped for direct runsc drive

A follow-up review replaced the provisioning implementation. The original
plan was the Docker Engine API (gVisor as the configured docker runtime).
The user asked whether a lighter, docker-nestable sandbox existed; the
comparison that settled it:

- **runsc is a standalone OCI runtime** — the backend can prepare a
  bundle and supervise `runsc run` as a child process, and the systrap
  platform needs no KVM, so managed hosts nest inside a containerized
  backend with zero host grants (no docker.sock, no DinD). User-namespace
  tools (bubblewrap/rootless podman) were rejected as a boundary (shared
  host kernel — the tier the baseline already bans); microVMs rejected
  (KVM requirement kills nesting).
- **Complexity accounting favored runsc-direct**: bundle prep is a
  config.json template + one shared read-only rootfs with runsc's own
  overlay (no per-instance unpack); lifecycle is plain child-process
  management (≈ the size of the Engine API client it replaces); the
  network rule was never free under docker either (same custom filtering
  work), so it is a constant, not a runsc cost. Docker's only real edge
  was cgroup convenience.
- **Image question resolved**: any distro userland works and security
  does not depend on it — the guest talks to gVisor's Sentry, never the
  host kernel; docker stays in CI as a build-time packager producing the
  rootfs tarball. Known cost is Sentry syscall compatibility (industry-
  accepted), not security.
- **Resource limits resolved**: memory/CPU/pids hard via a per-container
  cgroup-v2 subtree (delegation is a deployment prerequisite when
  nested); disk is soft — the idle sweep doubles as an overlay-size
  watchdog, breach freezes the host (same discipline as snapshot
  failure).

Outcome: runsc-direct is the only production implementation; there is no
Docker Engine API implementation at all. Design doc, boundaries doc, M7,
M10, and the M7 verification row updated accordingly. Deployment
prerequisites stated in the design: Linux-only provisioning, cgroup
delegation when nested, network policy at the backend's own boundary.

## M6 — Target switching + attachments (2026-09-01)

Status: done. Two commits: switching + `demi host` frame, then attachments.

### What landed

- **Switch mechanism** (`conversation/target-switch.ts`): one generic path
  behind `PATCH /api/conversations/:id {workspaceId}` — refused mid-turn via
  the new `AgentServer.sessionPhase()` (409 `turn_in_flight`), single winner
  via `ControlService.switchConversationWorkspace` compare-and-set on the
  current pointer (409 `switch_conflict` for the loser), departed target
  recorded in the prev slot. The prev slot lives as
  `conversations.prev_target_json` in control.sqlite — binding history
  belongs next to the binding, and `demi host` resolution reads the control
  plane anyway.
- **Context block** (`conversation/switch-announcement.ts`): injected
  through the harness `preamble` hook (the M0 mechanism, first product use —
  a `preamble` passthrough was added to `createCodingAgentHarness`). One
  announcement per switch via the prev record's `announced` flag, so it
  survives restarts; names both targets, the new start directory, states
  that no files moved and old paths are stale, gives the tar-pipe recipe,
  and adds the same-device note when the prev directory is still reachable.
- **`demi host` frame** (`managed/host-command.ts`, per the boundaries doc
  this module hosts the command group; the provisioner joins it in M7):
  status (current target + prev line), `prev shell -- <argv>`, `prev
  release`. The backend passes a commands *builder* to the coding harness —
  `AgentHarness.commands` context gained `agentSessionId` so product
  commands can close over their conversation. `createDemiCommand` gained
  `extraSubcommands` for the umbrella layering decided in the design round.
- **`prev shell` execution**: workspace prevs run by direct
  `host.process.spawn` on the device — streamed byte-faithful stdio, real
  exit codes, pre-start stdin bytes forwarded then stdin closed (post-start
  `shell_write` stdin is not forwarded — migration is a pull pipe). Virtual
  prevs run through an ephemeral prev-side `BashEnvironment`, where
  just-bash's portable set (full `tar` included) operates over the virtual
  `Host.fs`; output is buffered with a 256 MB cap (virtual trees are
  conversation-sized) and returned through the exec result's
  `binaryStdout`/UTF-8-delta byte-faithful contract. `Command` gained
  `restField` (raw argv after a literal `--`) for the `-- <argv>` form.
- **Workspaces CRUD** (`http/workspaces.ts`): list/create/rename/delete;
  creation validates device ownership; deletion is refused (409
  `workspace_in_use`) while conversations still point at the workspace —
  moving them is a target switch with its own turn-boundary rules, never a
  bulk pointer wipe.
- **Attachments**: `POST /api/attachments` stores bytes in the blob store +
  one metadata row (25 MB hardcoded cap). The send frame carries
  `{type:'ref', ref: <attachment id>}` media sources — a backend wire
  extension parsed by a zod schema in `conversation/attachment-refs.ts` and
  resolved to inline bytes in the scoped transport *before* the agent
  server validates the frame, so the agent protocol and providers see only
  the core media set. Missing/foreign refs degrade to a visible
  `[attachment … is not available]` text block. Frame rewriting became
  async (attachment reads); an in-order delivery chain keeps frame order.
  `POST /api/conversations/:id/workspace-files?name=…` writes dropped files
  into the target cwd over `Host.fs` (createParents, relative-path
  validation, 25 MB cap) and returns the absolute path for the input
  reference.
- **cwd on switch needed no work**: shells are born in `Host.defaultCwd`
  (workspace path for RemoteHost, `/workspace` for virtual), so the switch
  re-resolving the Host re-homes new shells automatically; the stale
  session-level cwd is covered by the context block until the next
  reconnect re-derives it.

### Correction during the round

The design round's "portable create-only `tar` registered into
virtual-target shells" was based on a false probe: a grep against the wrong
path of the nested just-bash repo (`packages/just-bash/src` instead of
`packages/just-bash/packages/just-bash/src`) concluded tar was absent, and
a redundant ustar implementation was briefly written. just-bash ships a
full `tar` (modern-tar; create+extract, gz/bz2/xz/zstd) already inside
`DEMI_PORTABLE_COMMANDS`. The reimplementation was deleted and the design
doc now states the fact; the virtual-prev pipe simply uses the existing
portable tar.

### Pitfalls

- StubProvider mid-turn gating: scripted arrays cannot hold a turn open;
  an inline provider awaiting a gate promise is the pattern for
  turn-in-flight tests (TS note: initialize the release fn to a no-op —
  assignment inside a Promise executor does not narrow).
- After reattach, `client.transcript()` is empty until the full-sync frame
  arrives — wait for blocks before asserting restored content.
- Verification coverage: `switch.test.ts` (virtual→real with pipe + context
  block, release closing the pipe, real→virtual with spawn-side prev,
  mid-turn 409, CAS single winner, offline chat + offline switch, workspace
  delete-in-use), `attachments.test.ts` (upload→ref→inline-at-provider,
  checkpoint round-trip, missing-ref placeholder, size/traversal refusals,
  drop visible to the agent shell).

### Command-surface consolidation (post-M6 review)

On review the command layering had one inconsistency: `todo` sat at the top
level while `host`/`agent` nested under `demi`. Consolidated: `todo` is now
a `demi` subcommand group, and the organizing rule is stated in the design
(§ The `demi host` command): everything Demi-specific lives under `demi` —
flat verbs = file operations, noun groups = platform domains; outside
`demi` is ordinary shell. `createCodingCommandRegistry`'s `includeDemi`
option died with the move. The `packages/web` dev-product suite has 5
pre-existing failures on this branch ("Provider … does not expose a runtime
factory") unrelated to this change — that package is replaced at M9.

Follow-up in the same review: the file verbs also moved into a noun group
(`demi file read/create/edit/patch`), so the rule has zero exceptions —
every `demi` subcommand is a domain group. Cost is one extra token per
file operation; bought: no dual-citizen layer to explain, and future file
operations have an unambiguous home.

## Execution layer review and owner decisions (2026-09-02)

Review of the M7 spike, then an item-by-item owner review of its
findings. The decisions are in the design records of this directory; this
entry keeps the evidence, the alternatives and the measurements.

### Design records split

`demi-next.md` grew past what one document can carry once the execution
layer was redesigned, and was split into this directory: `overview.md`
(stable core), `roadmap.md` (volatile), one record per subsystem
(`backend`, `storage`, `product`, `sessions-and-targets`, `commands`,
`shell`, `runner`, `managed-hosts`, `providers-and-vault`), and this log.
The review record `demi-next-execution-review.md` was folded into them and
deleted. The dated "verified facts" and audits from the original record are
evidence and live here (below); the facts that state current contracts were
kept in `runner.md`, `storage.md` and `providers-and-vault.md`.

### Milestones renumbered

M7 shell, M8 command system + loader + hostless, M9 runner on the shell +
deletion of just-bash and the command bridge, M10 access model + managed
hosts; the former M8–M11 became M11–M14. Ordering changed to strict
dependency order (lowest dependency first): the shell is built and judged
before anything is written on top of it; the loader is verified on the real
runtime before the runner moves. The M6 prev slot, `switch`, `release` and
tar-pipe migration are removed by M8–M10; the owner accepted the rework.

### Provisioning: runsc rejected, Firecracker adopted

runsc's "nests inside a containerised backend with zero host grants" claim
failed a privilege matrix inside Docker: gofer cannot start without both
`CAP_SYS_ADMIN` and `seccomp=unconfined`; the backend must vacate its root
cgroup; `memory.max` is not a hard limit without `memory.swap.max=0` (a
64 MB cgroup accepted a 200 MB allocation with 17 GB of host swap);
rootless mode ignores cgroup errors and has no Netstack. Firecracker under
nested KVM on Apple Silicon (Lima, `vmType: vz`): boot to init 0.65–0.70 s
with a minimal kernel, snapshot restore 12 ms, memory hard by construction.
Its costs, accepted: the guest kernel is ours to build; rootfs images are
block devices; `/dev/kvm` is required; jailer needs root at setup, confined
to a privileged helper.

### Persistence alternatives

- Memory snapshots rejected: sessions live in the backend, so nothing in VM
  memory is worth preserving; running processes already block hibernation.
- `kill -9` verified safe: filesystem clean after the kill; an unsynced
  file exists with empty content, a synced one is intact; the loss window
  is the writeback interval, empty by construction at hibernation.
- Persistent overlay (system layer in the home image) rejected in owner
  review in favour of a heavy preinstalled rootfs plus an ephemeral upper:
  the rootfs costs an owner nothing (shared, paged on demand, hot in the
  host page cache across VMs), while a persistent upper carrying a dpkg
  database makes the base image un-upgradable. The long tail installs into
  home (Linuxbrew for the apt-shaped tail).
- Storage: virtio-block passes no discard, so a live image is a high-water
  mark; offline `e2fsck -f` + `resize2fs -M` + `truncate` shrank 369 MB to
  69 MB in 0.11 s; re-grow 0.01 s. mkfs metadata at nominal 1/2/8/32 GB is
  33/66/69/261 MB — hence small nominal size plus online growth for empty
  homes, and untouched-skip on hibernate.
- Content-addressed storage for home images rejected: it manufactures
  superseded versions, orphans and a retention policy for an object that
  has one current version. Retention rule deferred by the owner.

### Ownership unit

Per-user "scratch" hosts (one VM and home per user, conversations as
directories) and per-workspace sharing were proposed to remove the
per-conversation VM cost for trivial work; the owner kept per-conversation
ownership. The trivial-work cost is instead removed by hostless execution:
a conversation gets a machine only at its first non-`demi` command.

### Execution model: hostless demi-only execution

The review proposed deleting the virtual target outright ("chat only
without a host"). Owner review found that most conversations only write a
file or run a query, for which a microVM is disproportionate, and that
what the model does without a host is entirely `demi` commands. Hence a
demi-only parser (tokens, heredocs, sequences; everything else refused)
and the loader running in the backend against the store-backed Host.
just-bash is still deleted: it was the bash parser and the in-process host
for registered commands, and a demi-only parser is ~200 lines. Pipelines
between real processes transited the backend under just-bash (`|`
implemented in memory); with real bash on the target the pipe is an OS
pipe.

`IN_PROCESS_PORTABLE_COMMANDS` is `echo`, `printf`, `pwd`, `alias`,
`unalias`, `history`, `help`, `time` — all bash builtins.

### Command system: one implementation, zero round trips

Three options were compared for `demi file *` on real hosts: a second
implementation in the runner's language (the review's original), relaying
every file operation to the backend (one implementation, two wire round
trips per operation — rejected by the owner as "silly"), and shipping the
implementation as a module to run where the files are. The third was
chosen and generalised into the `rpc` / `runtime` command kinds, the
command ABI and the loader. A Rust command tree with a WASM build for the
backend was considered when the runner was still Rust; it became moot when
the runtime question was reopened (next item).

### Runtime: Bun's cold start, LLRT, the shell

The review measured Bun's real runner at 2.7 s cold in a fresh microVM and
concluded Rust. Owner review asked whether a different JS runtime would
avoid the cross-language cost. A second measurement run on an M3 Pro (Lima
2.1.0 vz, Firecracker v1.16.1, CI kernel 6.1.155, 2 vCPU / 1 GB, a Bun
stub as the backend) reproduced the 6.24 s "runner online" as 6.3 s and
decomposed it:

| Segment | Time | Evidence |
|---|---|---|
| Bun runtime baseline | 0.10 s | hello, second exec |
| first executable mapping of the 100 MB binary | ~3.4 s | same file 4.95 s then 1.56 s; tmpfs copy 4.6 s then 1.6 s; host `drop_caches` no effect; equal page-fault counts, 310 extra major faults |
| evaluating the 4.5 MB bundle | ~1.45 s | second exec minus baseline; `--bytecode --minify` no gain (1.54 s) |
| runner `main` → online | 0.8–1.0 s | LocalHost 0.17, token read 0.17, TCP + upgrade 0.29, hello 0.05, config write 0.26 |

The review's attribution ("disk 0.95 s cold", "WebSocket 0.55 s") was
wrong on both counts: no disk effect exists, and the socket handshake is
0.3 s of which the stub answers in 1 ms. The first row scales with
executable pages touched (hello 0.9–1.5 s, runner 3.4 s, shell 0.1 s); the
mechanism consistent with every observation is per-page kernel work on the
first executable mapping (arm64 icache maintenance) amplified by nested
virtualization — inferred, not profiled; x86 unverified. The bundle row is
90 % just-bash (2.0 MB) and its dependencies (2.1 MB: yaml, domino, zod,
fast-xml-parser); the runner, protocol and host-local proper are ~50 KB.

Nested virtualization is the realistic deployment (cloud instances expose
KVM that way), so the numbers are not pessimistic. LLRT qualifies on size
but is experimental, Node-shaped and missing WebSocket/UDS; a shell of our
own on rquickjs is the same Rust work with a smaller, owned API.

Shell feasibility spike (150 lines of Rust, primitives `readFile`,
`writeFile`, `spawnTee`, `tcpGet`, native base64; a 386 KB ESM bundle of
the real `runner-protocol` codec plus zod 4), same guest:

| Measurement | Shell | Reference |
|---|---|---|
| binary | 1.45 MB (glibc, dynamic) | Bun 103 MB |
| hello, first / second exec | 0.12 s / 0.01 s | Bun 1.61 s / 0.10 s |
| protocol bundle module evaluation, first / second | 149 ms / 37 ms | — |
| zod encode of a byte-free message ×200 | 1 ms | — |
| zod decode of a `spawn` frame ×200 | 8–10 ms | — |
| tee 100 MB to a file with a 1 MB view | 83–96 MB/s | `head \| cat > file` in the guest: 78 MB/s |
| native base64, 16 KB | 300 MB/s | — |
| `bytesToBase64` from `@demicodes/utils` (pure JS) | 2.8 MB/s | native on Bun |
| TCP to the stub, first / second | 41 ms / 5 ms | — |

zod 4 runs unmodified in QuickJS; the tee saturates the guest's pipe; the
one order-of-magnitude loss is a pure-JS byte loop without a JIT, which
fixed the rule that byte-level work is a shell primitive. QuickJS lacks the
Web-platform globals and an event loop (the spike supplied a prelude and
ran synchronously). Decision: Rust writes the shell only; runner logic is
JS on it; no code generation. Spike artifacts: Lima instance `fc`,
`/opt/fc` inside it, `scratchpad/fc` and `scratchpad/shell`.

Other runtime numbers from the review's environment: Rust hello 0.030 s
cold (453 KB), Go hello 0.120 s (1.6 MB), Node hello 1.94 s, Node real
runner 2.59 s. Bun's default target is glibc and fails on alpine; the
shell is built static musl.

### Data carrier

zod 4.4.3 has no bytes type and `z.instanceof`/`z.custom` cannot export to
JSON Schema; walking the zod AST and the JSON-Schema → `cargo typify` route
both worked for discriminated unions and poorly for plain unions and custom
types. Moot with both protocol ends in TypeScript. Protobuf and Bebop
rejected as a second schema language. The exercise exposed two protocol
defects fixed in M9: the fs layer was untyped (`fs_call.args: unknown[]`,
`fs_result.result: unknown`), and `fs_result` shared `type` across ok and
error. The zero refinements in the RPC schemas (all 13 in the codebase are
on Web API bodies) mean nothing beyond structure and presence is validated
on the wire.

### Wire audit

Backend → browser inlined media: `sync_transcript` → `sendTranscriptReset`
→ `structuredClone` kept `Uint8Array` → `stringifyPortableJson`;
`externalizeBlockMedia` ran only on the persistence paths. Giant stdout:
the runner pumped every chunk uncapped, `RemoteSpawn.chunks` was unbounded,
the backend dropped the capture and `SIGKILL`ed at 64 MB after the bytes
had crossed the wire, then wrote the artifact back over `host.fs.writeFile`.
Both fixed by the bytes rule and the tee. The channel-layer risk (control
frames queued behind data frames) is recorded in `runner.md` with its
precondition.

### Managed-host security items reassessed

- Device token on the kernel command line, readable by guest processes:
  judged not a material widening (the VM is the trust boundary; a process
  inside already sees everything the backend sends). Hiding `/proc/cmdline`
  rejected (bootargs also appear in the device tree and dmesg). A one-shot
  bootstrap exchange rejected as protocol weight for a case that no longer
  exists once one live connection per token is enforced. Source-address
  binding rejected: it would force managed runners onto a private path
  through the tap gateway.
- Egress policy vs backend reachability: resolved by having managed runners
  dial the public backend URL like user hosts; no carve-out.
- Artifacts at hibernation: not uploaded (owner decision); offline hosts
  show a notice, hibernated managed hosts wake on open.
- Auto-grant on switch: kept as decided; revocation is a UI item.

### Verified facts carried over from the original record (2026-08-31)

Established by code reading and local-mock experiments; no real provider
endpoint was contacted.

- Every HTTP provider runtime accepts `baseUrl` + extra headers and its
  full endpoint surface follows `baseUrl`; auth-plane endpoints are
  hard-coded. API-key options on openai/anthropic/google are resolver
  functions.
- Codex: WS transport is a scheme swap on the same host/path; its auth
  relies on Bun's `WebSocket(url, {headers})`; `x-codex-*` quota rides on
  inference response headers.
- Claude Code CLI 2.1.220: single request class via base-URL override;
  env-token adoption; harmless direct CONNECT attempt.
- `Host` is platform-neutral and enforced (`root-entry.test.ts`); in-memory
  Host/store shapes exist as test doubles.
- `AgentSession.fromCheckpoint` restores from `{transcript, state, …}` and
  force-completes executing tool calls; session ids are client-owned with
  takeover semantics.
- Host portability: all `HostFileSystem` methods async with plain-data
  I/O; `HostCwd` fd anchors and sync `Host.identity` have designed answers.
  Multi-Host-per-session exists and is tested (`host-routing.test.ts`). No
  remote Host existed before M1.
- The DB-backed `HostStore` must re-provide `LocalHostStore`'s `writeJson`
  atomicity and serve `list` + bulk reads efficiently.

## tinybash scope: bash-usage corpus (2026-09-02)

To place tinybash's boundary on evidence rather than taste, every `bash`
tool call in the local Claude Code transcripts on the design machine was
extracted and classified (`~/.claude/projects/**/*.jsonl`; 165 transcripts,
107 projects, 6 120 calls). The script tokenizes each call (quotes and
heredoc bodies stripped), flags the constructs used, and records the
programs invoked; it prints frequencies only.

**Bias notes.** The corpus is a coding agent *with* a machine, so it says
what shapes the model habitually writes, not what a hostless conversation
would write once told it has no machine. That environment also instructed
the agent to read files with `cat`/`head`/`sed -n` instead of a file tool,
inflating `head` and pipes; and a quarter of the calls are this
repository's sessions, heavy in Lima and systemd operations. Excluding
this repository (4 472 calls) moves every ladder step by under two points,
so the shape is robust.

Construct frequency (share of calls using it):

| Construct | Share |
|---|---|
| more than one statement | 87 % |
| pipe | 68 % |
| any redirection | 46 % — `2>&1` 34 %, `/dev/null` 14 %, write to a file 2.5 %, read from a file 0.3 % |
| `;` | 40 % |
| `&&` | 38 % |
| `cd` | 27 % — as a `cd X && …` prefix 23 % |
| `~` | 19 % — all `~/` paths |
| glob | 9 % |
| heredoc | 8 % |
| `$NAME` | 8 % |
| assignment | 5 % |
| `$( )` / backtick | 3.5 % |
| control flow | 3.3 % |
| `\|\|` | 2.2 % |
| subshell | 0.4 % |
| background `&` | 0.2 % |
| here-string | 0.1 % |
| `$` in a bare-delimiter heredoc body | 0.03 % |

Pipes: 66 % of all calls pipe only into text filters; 2 % pipe into a
real tool. Downstream of a pipe: `head` 40 %, `tail` 23 %, `grep` 23 %,
`sort` 2 %, `sed` 2 %, `awk` 1.4 %, `python3` 1.3 %, `cut` 1.3 %, `wc`
0.8 %.

Cumulative coverage, ordered so each step adds the cheapest remaining
construct (all projects / excluding this repository):

| Subset | All | Excl. |
|---|---|---|
| one simple command, quotes | 9.4 % | 7.6 % |
| + newline `;` `&&` `\|\|` | 16.4 % | 12.1 % |
| + heredoc, here-string | 17.6 % | 13.4 % |
| + `cd` | 20.1 % | 15.8 % |
| + pipes into text filters | 37.5 % | 30.5 % |
| + `2>&1`, `/dev/null` | 63.3 % | 57.6 % |
| + `~/` | 78.5 % | 78.3 % |
| + `$NAME`, assignments | 82.7 % | 82.6 % |
| + globs | 90.3 % | 90.0 % |
| + `> file`, `< file`, pipes into tools | 94.0 % | 93.9 % |
| + `$( )`, backticks | 96.4 % | 96.5 % |
| + control flow, subshells, `&` | 100 % | 100 % |

Programs (share of calls invoking): `grep` 48 %, `head` 41 %, `cd` 27 %,
`tail` 24 %, `echo` 22 %, `bun` 21 %, `git` 17 %, `sed` 16 %, `ls` 10 %,
`python3` 9 %, `cat` 6 %, `bunx` 4 %, `sleep` 3 %, `curl` 2.5 %, `wc`
2.3 %, `sort` 2.2 %, `find` 1.9 %, `awk` 1.7 %, `rm` 1.7 %, `cut` 1.3 %,
`rg` 1.2 %, `mkdir` 0.6 %, `cp` 0.5 %. Calls whose programs are all
file/text/fs-mutation/shell builtins: 42 %; calls invoking at least one
real tool: 58 %. With every grammar level and only builtin-class programs,
coverage caps at 41.6 % — the program axis, not the grammar, bounds what a
hostless shell can ever run, and that remainder is exactly what
auto-provisioning serves.

**Decisions taken.** Grammar through file redirections (94 % of shapes),
stopping before command substitution and control flow (the last 6 % costs
an interpreter). A closed builtin set with whitelisted flags — grep, head,
tail, cat, echo, ls, find, wc, sort, uniq, cut, tr, mkdir, rm, mv, cp,
touch, pwd, true, false, test, cd — and `sed` only for printing line
ranges, because `s///` was the largest divergence source in the previous
portable-command effort. GNU coreutils is the reference (managed hosts are
Linux); the equivalence corpus runs against real bash + GNU tools in a
Linux container in CI. Considered and rejected: stopping at roots-only
(20 %; every `grep … | head` would provision a machine), and going to
control flow (an interpreter, which is just-bash again).

### Upgrade policy: silent, always (2026-09-02)

Three ways to move a hostless conversation onto a machine were compared:
the agent asks (a `demi host` verb plus prompting about the two-tier
world), the user is asked (a permission-style pause in the UI), or the
backend does it silently on the first script tinybash cannot run. Agent-
initiated was rejected: it puts the decision on the weakest component (M6
showed model-driven switching to be the least reliable part of the design),
costs retry loops when the model keeps trying `python3`, and would need
every library embedder to reproduce the prompting. A per-instance policy
knob (auto / confirm / never) was proposed and rejected by the owner: one
behaviour, not three. Decision: silent always, with the environment made
identical on both sides so nothing needs to be announced — same user and
home path, files placed at their own paths, shell state carried over, GNU
output on both sides — and no context block. tinybash's refusal messages
therefore surface only for embedders with no machine; in the product,
grammar and program limits are the same "outside the subset" case. A
deployment with neither managed hosts nor local execution enabled is a
hard failure surfaced to user and agent, not a policy.

### Guest user and deployment requirement (2026-09-02)

- Jobs on a managed host run as the guest user `demi` with passwordless
  `sudo`, not as root: the runner is PID 1 as root and drops to `demi` for
  every spawn. Root-only would break the tools chosen for the long tail
  (Linuxbrew refuses root) and would leave files root-owned; a user
  without sudo would make `apt` fail although the ephemeral upper exists
  for it. The user boundary is for compatibility, not security — the VM is
  the boundary.
- The "backend's own machine as a runner" path is deleted. Managed hosts
  are a deployment requirement; backends without `/dev/kvm` are unsupported
  for now, with support for such machines a later design item. Consequence:
  there is always a machine to upgrade to, so the hostless design has no
  "no machine" branch at all.

### The upgrade condition and the hostless namespace (2026-09-02)

Aligning only the home path was found insufficient for a silent upgrade:
`cat /etc/os-release` or `ls /usr/bin` would answer differently hostless
and on a machine. Decision: the hostless filesystem is exactly
`/home/demi` and `/tmp` (plus `/dev/null`); any absolute path outside that
namespace — in builtin arguments, redirections, `cd`, glob expansions or
path-typed root-command arguments — is an upgrade condition decided at
parse time, alongside grammar, programs and flags. Path-typed arguments
are marked in the manifest schemas so the loader and tinybash can check
them. The storage quota is the one run-time-only failure and is an error,
not an upgrade. Metadata (mode, mtime, symlinks, case sensitivity, owner
`demi`), the environment table and GNU output are aligned on both sides;
the "split equivalence" test defines silence.

### Hostless files: tree + blobs → home image (2026-09-02)

The hostless filesystem was specified as `host_store` rows in the
conversation database, with "the backend writes the files into the home"
at upgrade — bytes in a database, against the storage rule, and an
unspecified placement mechanism (a guest-side copy or a privileged
mount). Decision: a `files` tree table (path, kind, mode, mtime, size,
sha256, symlink target) in the conversation database with contents in
the content-addressed blob store; at upgrade the tree is materialised to
a directory and `mke2fs -d` produces the populated home image without a
mount, root or guest cooperation; the tree rows are then deleted, blobs
stay. One direction, once. The tee/artifact mechanism was considered and
rejected for hostless files: it is for command output on targets, and
hostless output is bounded and lives in the transcript.

### Naming: "artifact" → "command output" (2026-09-02)

The tee's full-output files had been called artifacts. In agent products
that word names the agent's deliverables, so it is now reserved for that
and the files are **command outputs**: `commandOutputDir`,
`output_upload`, `GET …/outputs/:ref`, output fetch. The code's
`commandArtifactsDir` and `command-artifact-store` are renamed with the
M9 runner rewrite.

### Naming: host vs guest (2026-09-02)

"Host" was being used in two senses — Demi's execution target (user host,
managed host, the `Host` contract, hostless) and virtualization's machine
that runs a VM. Decision: "host" is Demi's sense only; the VM is the
*guest* and the machine running the backend and Firecracker is the
*backend machine*. A vocabulary table (target / device / host / guest and
backend machine) is in `overview.md`; `managed-hosts.md` was reworded.

## Shell API shape (2026-09-02)

Decided in discussion, recorded in `tinyjs.md`: the shell's job is to run
one bundled ESM module plus `import()` of absolute paths for `runtime`
command modules. The API is a set of built-in modules under the `tinyjs:` scheme,
resolvable only from the embedded bundle and imported only by
`@demicodes/host-tinyjs` (a global object was rejected: any code, including
a downloaded command module, could reach it); integer handles with explicit close; pull-model
reads; Node errno codes. Raw TCP and TLS primitives were dropped — the
only network users are the WebSocket to the backend, HTTP for uploads and
transfers, and the UDS relay, so those are the exposed level. pty, servers,
mount/netlink, watch, workers, wasm and compression are explicitly outside
the first version.

## Shell spike 2: LLRT module crates (2026-09-02)

Question: assemble the shell from LLRT's per-module crates (`llrt_fs`,
`llrt_net`, `llrt_fetch`, `llrt_timers`, `llrt_url`, …, rquickjs 0.11,
Apache-2.0) instead of writing the primitives. Built in the Lima `fc`
instance (`/opt/fc/shell2`), tested on the Lima host and inside a
cold-cache Firecracker guest alongside spike 1's bare-rquickjs shell.

Measured:

| | spike 1 (bare rquickjs) | spike 2 (LLRT crates) |
|---|---|---|
| binary | 1.45 MB | 5.5 MB (3.15 MB without fetch: hyper + rustls are 2.35 MB) |
| hello first exec, guest, cold | 0.18 s (same boot) | 0.25 s |
| hello second exec | 0.02 s | 0.02 s |

Worked unchanged: timers (ordering, interval, 2 ms sleep error), fs
operations incl. symlink/chmod/readdir types, UDS listen/connect by path,
HTTP GET/PUT and HTTPS via ring, `import()` of absolute paths, a resolver
guard that refuses `fs` etc. from file-loaded modules while the embedded
entry (named under `/embedded/`) sees them — this is the mechanism
`tinyjs.md` specifies for keeping `tinyjs:*` private.

Fell short: no errno `code` on any fs or net error (message text only);
socket `write()` returns nothing and there is no `drain`, the queue is
unbounded; fetch has no ReadableStream — `Response.body` is undefined and
request bodies must be in-memory bytes or Blob (guest GET of 16 MB took
1.36 s); `readFile` of 50 MB took 2.2 s in the guest against 0.25 s for a
chunked read with a reused 1 MB buffer and 0.02 s for `cat`; `crypto`
and `structuredClone` absent; the build needs cmake for the compression
feature.

Environment fact, independent of the runtime: first touch of 50 MB of
fresh memory in the guest is 0.93 s, the second 0.16 s — nested
virtualization makes fresh pages expensive, which is why every large
buffer must be allocated once at its known size in Rust.

Decision: everything is ours. What LLRT provided correctly is the trivial
part; everything with semantics we depend on needs rewriting anyway.
Considered and closed in the same discussion: a C shell on QuickJS +
libuv + libcurl (no memory safety for a PID 1 that faces the network,
static musl builds of libcurl + TLS are the harder packaging problem, and
libcurl's one real advantage — proxy handling — is a small feature we
implement in `tinyjs:net`); tokio's and rustls's reputations (the
`Send` friction disappears on a current-thread runtime, and rustls on the
`ring` backend with a platform verifier on user hosts is the settled
choice). Size levers recorded in `tinyjs.md` Packaging: lazy network
initialisation, per-package opt-level, no hyper/tungstenite, ring, trimmed
tokio features, QuickJS without bignum; UPX and nightly build-std rejected.

## M7 — Shell (2026-09-02)

Status: delivered on `feat/demi-next`. Crate at `packages/tinyjs`;
`cargo test --features conformance` runs the primitive conformance suite
(`conformance/*.mjs`, embedded as the bundle under that feature so it sees
`tinyjs:*`) with the ports, test CA and Bun stub it needs.

### Acceptance, measured

| | |
|---|---|
| Conformance | 54/54 on macOS arm64; 54/54 macOS x86_64 under Rosetta; 46/46 Linux aarch64 musl inside the Firecracker guest as root (stub-backed net cases skipped: no Bun in the guest) and 43/43 in the Lima VM; Linux x86_64 musl builds (`cargo zigbuild`) but has no machine here to run on |
| Binary | 2.3 MB macOS arm64, 2.6 MB Linux aarch64 musl, 3.0 MB Linux x86_64 musl, 2.6 MB macOS x86_64 |
| Command-mode hello, guest, cold cache | 0.18 s first execution, 0.02 s second; no network code on the startup path (the TLS config and connectors are built on first use) |
| Tee, 100 MB from `/dev/zero` to a file in the guest | 126 MB/s; `head \| cat > file` baseline 86 MB/s with fresh pages and 116 MB/s warm |
| runner-protocol codec bundled with Bun, loaded on the shell | encodes and decodes; schema rejects bad frames |

Cross builds: `cargo zigbuild --release --target
{x86_64,aarch64}-unknown-linux-musl --features guest-roots` from macOS
(zig 0.15, cargo-zigbuild 0.23); macOS x86_64 with the rustup target. The
guest fixture is the Lima `fc` instance's Firecracker setup from the
spikes, with `/opt/tinyjs/{tinyjs,tinyjs-conf}` and a
`tinyjs` case in the rootfs `init.sh`.

Left for later milestones: the stub-backed net cases on Linux (install
Bun in the fixture), a Linux x86_64 run, and the `ENOTFOUND` case (this
network sinkholes unresolvable names).

### Landed

- Event loop, module loader (`/embedded/*` table, absolute and relative
  file paths, `tinyjs:*` only from the embedded bundle, `import.meta.url`),
  `ShellError` with errno codes, the handle table, standard globals,
  `tinyjs:fs`, `tinyjs:bytes`, `tinyjs:runtime`, the entry-mode
  skeleton. 31 conformance cases pass on macOS arm64. Release binary
  1.5 MB before any network code; command-mode hello starts in 7 ms.

### Pitfalls

- `AsyncRuntime::idle()` holds the runtime lock for its whole duration, so
  nothing outside it (a signal task) can enter the context while the loop
  waits. The loop is therefore one `async_with` for the life of the
  process, with its own liveness count (`State.active`: in-flight IO,
  live timers, child waits) deciding when to exit; signal handlers do not
  count, as in Node.
- `while let Some(t) = queue.borrow_mut().pop()` keeps the `RefCell`
  borrowed through the loop body; a timer callback that schedules another
  timer panicked. The borrow is now scoped before the callback runs.
- rquickjs does not set `import.meta.url`; the loader sets it from the
  module name after `Module::declare`.
- QuickJS `Error.stack` holds only the frames; the harness prints
  `name: message` itself.

### Decisions taken while implementing

- Regular files and the standard streams are `Arc<std::fs::File>` driven
  through `spawn_blocking` with a cloned descriptor, so concurrent
  operations on one file need no bookkeeping; pipes and sockets are tokio
  streams with one read and one write in flight (`EBUSY` otherwise) and
  `close` cancelling a pending operation with `ECANCELED`.
- `console`, `queueMicrotask`, `TextEncoder`/`TextDecoder`, `URL`,
  `AbortController` and `structuredClone` are prelude JS over native
  transcoders; only per-byte work is Rust. `tinyjs.md` says the same.
- MessagePack follows `@msgpack/msgpack` defaults: integral numbers as
  ints, `Uint8Array` as bin, `Date` as the timestamp extension, `undefined`
  as nil.
- `tinyjs:net` landed on crates, not hand-written protocols (owner
  decision 2026-09-02: "we do not implement protocols ourselves"):
  `tokio-tungstenite` for WebSocket, `hyper` + `hyper-util` for HTTP/1.1
  and the `CONNECT` tunnel with proxy-environment matching,
  `tokio-rustls` with the platform verifier (or `webpki-roots` under the
  `guest-roots` feature), UDS listen/accept/connect. The hand-written
  base64 and MessagePack decoder went the same way (`base64`, `rmpv`).
  Release binary 2.3 MB with everything in. `cargo test --features
  conformance` provisions free ports, an openssl test CA and the Bun stub
  (`conformance/stub.ts`: HTTP, WebSocket, CONNECT proxy) and runs the 51
  cases, including https/wss through the proxy.
- Pitfalls in net: closing a WebSocket the peer already closed makes
  tungstenite report `SendAfterClosing`, which the shell treats as a
  successful close; rustls reports handshake failures as `InvalidData`
  (`EINVAL` by kind), remapped to `EPROTO`; Bun's WebSocket server drops
  echoes past its `backpressureLimit`, so the stub raises it; a `.invalid`
  hostname resolved on this network (sinkholed DNS), so the suite has no
  `ENOTFOUND` case.
- `tinyjs:process` landed: spawn over `std::process::Command` with the
  pipes handed to tokio (`pipe::Receiver`/`Sender`), the shell's own
  SIGCHLD reaper (`waitpid(-1)`, which is also the PID 1 orphan reaper),
  wait with tee byte counts, kill with process groups, the bounded view.
  41 conformance cases pass.
- macOS bash 3.2 reports exit 0 for a group-killed `sh -c "sleep & sleep;
  wait"` about a quarter of the time (reproduced with Python's `killpg`
  outside the shell); the conformance case asserts the grandchildren died
  through pipe EOF and accepts either status.

### Naming: the shell → tinyjs (2026-09-02)

`@demicodes/shell` is already the Host contract and the agent's command
environment, so calling the Rust binary "the shell" gave one word two
meanings and made `@demicodes/host-shell` read as the Host over that
package. The binary is now **tinyjs**, pairing with tinybash: the bash the
agent runs in and the JS the runner and commands run on. Renamed
throughout: crate `packages/tinyjs`, modules `tinyjs:*`,
`@demicodes/host-tinyjs`, `docs/demi-next/tinyjs.md`, the
`TINYJS_CONFORMANCE_*` variables. Rejected: `demi-js` (fine but
unpaired), `demi-vm` (collides with the microVM vocabulary), `demi-core`
and `demi-runtime` (taken).

### tinyjs entry: packed binary instead of a compiled-in bundle (2026-09-02)

Owner decision: no `include_str!`, no cargo build to change the bundle;
follow Bun's `--compile` and Deno's `compile`, where a deliverable is a
prebuilt runtime plus a bundle. Landed in `packages/tinyjs`: the bare
binary runs `tinyjs <entry.mjs> [args…]` with the entry's directory as
`/embedded/` (this is how the conformance suite runs now; the
`conformance` feature and `embedded.rs` are gone), and `tinyjs --pack`
injects a bundle with `libsui`, the Rust crate Deno wrote for exactly this
after hitting the same problems. `--bin` packs another platform's bare
binary; all four targets were packed from macOS and run (macOS x86_64
under Rosetta, Linux aarch64 in the Lima VM).

Mechanisms tried before libsui, in order:

- Appending bytes plus a trailer after the executable: runs on both
  platforms, but `codesign` refuses the file ("main executable failed
  strict validation"), which rules it out for macOS distribution.
- `postject` (LIEF, what Node uses): on macOS it mis-relocated the Rust
  binary's `__thread_bss` section and dyld refused to load; on the static
  musl ELF it reported success and wrote no note at all.
- A reserved 8 MiB slot written in place: worked everywhere and signed
  cleanly, but costs its capacity in file size and is a private format;
  rejected by the owner as abstract and unreasonable. It also showed that
  placement matters in the guest: the slot in `.rodata` ahead of the code
  made the cold first execution 0.25 s instead of 0.18 s.

Owner decision the same day: the packed bundle is always QuickJS bytecode,
no source option (Bun and Node default to source with an opt-in; we do
not). `--pack` compiles with this tinyjs's interpreter, with the real
loader installed because declaring a module resolves its imports, then
injects the bytecode; `Module::load` reads it from the mapped image at
start. Pitfall: libsui does not check Mach-O header padding, and the debug
binary had 0x68 bytes after its load commands, so the injected segment
command overwrote the first 16 bytes of `__text` (SIGILL at
`0x1000009c0`); release binaries happened to have room. macOS targets are
now linked with `-headerpad 0x2000` (`.cargo/config.toml`) and `--pack`
checks the padding of whatever it is given. All four targets were packed
from macOS and run.

Verified upstream (2026-09-02): this is libsui's own open bug, not a
misuse — `denoland/sui#82` ("Resulting binary (often) failing with SIGILL
on MacOS", 2026-08-16) and PR `#83`, unmerged, which reads exactly like our
crash: `build` pays for the 152-byte load command by dropping that much of
the padding after the load commands without checking it exists, and a
stock `cargo build` binary often has 48 bytes. The PR's author adds that
release binaries are corrupted identically and only look fine because a
different function sits at the start of `__text` — so our release build
"working" was luck, and the `-headerpad` link flag plus the pre-check are
the correct mitigation until upstream shifts the image itself. The same PR
found a second defect: with `LC_DYLD_CHAINED_FIXUPS` libsui leaves
`seg_count` one short and `dyld_info -fixups` rejects the output (dyld is
lenient, so `deno compile` binaries run anyway). tinyjs is linked with
`LC_DYLD_INFO_ONLY`, `dyld_info -fixups` accepts the packed binary, and
`-fixup_chains` must not be added to the link flags while that bug is
open. Deno's own `.cargo/config.toml` sets no `-headerpad`.

`cargo test` runs the conformance suite on the bare binary and a
packed-binary case (`--pack`, strict signature verification on macOS, run
through `demi` and `demi-runner` symlinks, arguments passed through
untouched). `TINYJS_CA_FILE` replaces the TLS root store with a PEM file
(as `SSL_CERT_FILE` does for OpenSSL); the suite uses it for its stub CA.

## M8 — Command system, loader, hostless execution (2026-09-02)

Status: in progress on `feat/demi-next`, split into two checkpoints.

### Checkpoint 1: `@demicodes/tinybash` as a standalone package

Owner decision (2026-09-02): tinybash depends on nothing in M8 but the
two `@demicodes/shell` types it already has (`HostFileSystem`,
`CommandIO`), a `dispatch` callback and a per-root path-argument function,
so it is built and accepted first, on its own verification: the grammar
table, the builtin table and the equivalence corpus. The split-equivalence
cases and the backend integration belong to checkpoint 2 (command tree,
ABI, manifest, loader, hostless conversation, second root).

Interface adjustments made while starting (in `tinybash.md`): `dispatch`
receives stdin, cwd, env and signal alongside the writers, because that is
what a root command's `ctx` is built from; the per-root "which arguments
are paths" is a function of the argv, built by the loader from the
manifest's path marks, because which argument is a path depends on the
leaf the argv selects and tinybash must not parse root arguments itself.

Equivalence corpus mechanics: scripts and a fixture tree under
`packages/tinybash/src/__tests__/corpus/`, root commands stubbed as shell
scripts that record argv and stdin; golden outputs from real GNU bash are
generated by a script and committed, so `bun test` compares tinybash to
the goldens on every platform. The generator runs `bash` directly on
Linux and `limactl shell fc -- bash` from macOS (the Lima instance has bash
5.2.21, coreutils 9.4, grep 3.11, sed 4.9 and mounts the repository); a
Linux-only test re-derives the goldens and fails if they drifted.

Checkpoint 1 result (2026-09-02): `@demicodes/tinybash` landed
(`3dc26b2`). 309 corpus cases plus the refusal table; on Linux (the Lima
instance, bun 1.4) 330/330 pass with the goldens re-derived from bash
5.2.21 / coreutils 9.4 in the same run; on macOS 326 pass and the four
filesystem-dependent `ls -l` cases are skipped. Package layout is the one
in `tinybash.md` (`grammar/`, `outside/`, `exec/`, `builtins/`).

Pitfalls met:

- Golden file names collided on the case-insensitive macOS filesystem
  (`grep-c` / `grep-C`, `echo-e` / `echo-E`) and the two cases silently
  shared one golden; names are now unique case-insensitively and a test
  enforces it.
- Concurrent `limactl shell` sessions interleave their stdin/stdout, so
  the generator runs one bash at a time by default (8 s for the corpus).
- A symlink's own mtime cannot be set through `HostFileSystem` (no
  `lutimes`), so no case lists a symlink long-form with its time.
- bash 5.2 enables `globskipdots`: `.*` no longer matches `.` and `..`;
  tinybash follows 5.2.
- Fixture modes are explicit on both sides because the two machines have
  different umasks.
- GNU details worth knowing when extending the builtins: `wc` prints the
  `total` line whenever more than one operand was given, even if one
  failed, and reports a directory as zero counts plus an error; `uniq`
  always terminates its last line; `head`/`tail` say "cannot open 'x' for
  reading"; `sort` says "cannot read: x"; `cp` into a missing directory
  must be caught before `Host.fs.cp`, which creates parents; `ls` without
  a terminal prints one name per line, so the column layout never
  arises in a tool call.

Decisions taken while implementing:

- Unquoted parameter expansions are word-split on the default `IFS`, as
  bash does; the doc originally did not say, and without it `X="a b";
  cmd $X` would not be bash.
- `{}` and `{x}` are literal (bash expands only `{a,b}` and `{1..3}`), so
  `find -exec … {}` is refused for `-exec`, not for the braces.
- `head -N` / `tail -N` (the obsolete GNU spelling) are accepted: the
  corpus showed models write `head -20` about as often as `head -n 20`.
- `find` and `grep -r` visit directory entries in name order; GNU's order
  is the filesystem's and is not something a script can rely on, so the
  corpus pipes them through `sort`.
- `ls -l` block totals are computed as ext4 would report them for
  ordinary files (4 KiB blocks, none for empty files or fast symlinks);
  the Linux run confirms the numbers for the fixture.

### Checkpoint 2 — command tree, loader, hostless execution

Order: (1) command tree types and the ABI in `@demicodes/shell`; (2)
`@demicodes/command-loader`; (3) `demi file` as runtime modules, `todo`
and `host` as `rpc`; (4) the hostless path in the backend through
tinybash and the loader; (5) a second root, the embedding example and the
acceptance tests. Each step lands with its docs and one commit.

Decisions taken before writing code:

- A `runtime` leaf carries its module as **text** (`module: string`), not
  a file path: the tree is declared in TypeScript and its source paths do
  not survive packaging, while the manifest hashes content anyway. The
  text comes from `import x from './read.command.ts' with { type: 'text' }`.
  Verified: Bun honors the attribute; tsdown/rolldown does not (it neither
  passes import attributes to plugins nor recognizes `?text`), so a
  `load` plugin serves `*.command.ts` by name; TypeScript types the import
  as the module (a wildcard ambient declaration does not override a real
  file), so `runtimeModule()` is the single conversion point and checks
  for text at runtime.
- The manifest build transpiles with Bun in the backend composition root;
  `@demicodes/command-loader` stays pure JS.
- `CommandResult` is `{ exitCode }`; the file commands' diff `metadata`
  had no consumer outside their own test and is gone.
- The hostless environment (tinybash + loader behind the `shell_*`
  tools) lives in the backend, the only hostless embedder; the shell
  package is due to shrink in M9.

Step 1 done — the tree and the ABI (`@demicodes/shell`): `Command` is
`CommandGroup | RpcCommand | RuntimeCommand`; groups navigate only (a
group with nothing after it prints its help), so the two dual-mode nodes
became `demi agent spawn` and `demi host current`. `CommandContext`,
`CommandResult`, `DispatchIO`, `pathArg` / `isPathArg`, `runtimeModule`
and `loadCommandModule` (a `blob:` import) live in `command-abi.ts`;
`runRegisteredCommand` runs both kinds and takes stdin as a stream, so
the same entry serves the just-bash bridge today and the loader next.
`demi file read/create/edit/patch` are `*.command.ts` modules under
`coding-agent/src/commands/file/`, imported as text; the tsdown build of
`coding-agent` runs `commandModulesAsText` from `@demicodes/shell/build`
and the built bundle carries the four module texts inline (verified).
The file commands' diff metadata and the shell's `commandMetadata`
plumbing are gone. tinybash's byte-stream helpers moved to
`@demicodes/utils` (`bytesStream`, `collectBytes`, `emptyByteStream`,
`concatByteStreams`, `toBytes`) and it takes `DispatchIO` from shell.

Step 2 done — `@demicodes/command-loader`: `buildManifest(roots, {
transpile })` (zod → JSON Schema per leaf, module text → transpiled JS
under its SHA-256, a value import fails the build, manifest hash over the
whole), `parseManifest` (zod schema for the wire), `treeFromManifest`
(JSON Schema → zod with `z.fromJSONSchema`, path marks and descriptions
intact, so help from a reconstructed tree equals help from the declared
tree — a test asserts it), `createLoader({ source, host, rpc? })` whose
`dispatch` is `runRegisteredCommand` on the reconstructed tree,
`inProcessRpc` for the backend, `rootPaths` from the path marks. The
manifest build's transpiler is injected rather than imported so the
package stays pure; `RootPaths` moved to the shell ABI since both
tinybash and the loader speak it. A comment-only change to a module is
the same hash: the hash is over what runs.

Steps 3 and 4 done — kinds in the trees and the hostless path:

- `@demicodes/shell`: `ShellEnvironment` (the contract behind the
  `shell_*` tools) with the API types in `shell-environment.ts`, and the
  command records, status views, artifact persistence and the
  final-stream (text / binary) boundary in `command-records.ts`, shared
  by `BashEnvironment` and the hostless engine — one place computes what
  `shell_status` shows, whichever engine ran the script.
- `@demicodes/agent`: `AgentServer` takes a `shellEnvironment` factory
  (`ShellEnvironmentFactory`, default `defaultShellEnvironment` =
  `BashEnvironment`); `LiveSession` and the child supervisor build every
  per-Host environment through it and are typed on the interface.
- `@demicodes/backend`: `HostlessEnvironment` — tinybash over the
  conversation's `VirtualHost`, roots dispatched through the loader with
  `inProcessRpc`, the manifest built per session with Bun's transpiler
  (memoized per module text). Observation window, `shell_status`,
  `shell_write` (the script's stdin, steering `rpc` handlers such as
  `demi agent spawn`), `shell_abort`, capture limit, binary final streams
  and artifacts all behave as with bash. A script outside the subset
  runs nothing and answers with tinybash's refusal line on stderr, exit 2
  (the managed-host hand-over is M11). `createBackend` picks it for
  every `VirtualHost`.
- The hostless home is `/home/demi` (`HOSTLESS_HOME`, namespace
  `['/home/demi', '/tmp']`), as `sessions-and-targets.md` says;
  `VirtualHost.ensureLayout` creates the declared directories.
- tinybash: `runTinybash` takes the script's `stdin`; a root command whose
  stdin is not redirected receives it as `DispatchIO.stdinStream` (shared
  iterator, `shareByteStream`), and the loader passes it to `rpc` handlers
  as `stdinStream` and to `runtime` modules after the pipe.

Pitfalls met:

- A blanket `/workspace` → `/home/demi` rename in the backend tests also
  rewrote `/api/workspaces`; the rename is by word boundary now.
- `printf` is not in the tinybash subset, so two backend tests that used
  it to seed a file were outside the subset; they use `echo -n`. Models
  write `printf` often — a subset question for the user, not decided
  here.
- Artifact writes are chained asynchronously behind the status; a test
  that reads `stdout.bin` right after the exited status polls for it.
- `$?` is outside the grammar, so a test cannot echo an exit status; it
  observes the status the environment reports instead.

Step 5 done — the second root and the embedding example: `scout` is the
second root in the loader tests and the hostless test (an `rpc` leaf that
is steered and aborted through `shell_write` / `shell_abort`), and
`examples/embed-commands/` embeds `demi` plus a `scout` root with only
the loader and a Host, then puts tinybash on top for a hostless shell
(`bun run examples/embed-commands/main.ts`). `zod` is a root
devDependency so examples can declare input schemas.

Checkpoint 2 result: typecheck clean; scoped `bun test` green for utils,
shell, host-virtual, command-loader, tinybash, coding-agent, agent,
backend, host-local (one pre-existing timing-sensitive shell test,
"flushes redirected foreground output on shell_abort", is flaky under
load on this machine and unrelated).

Left for later, by design:

- Runtime-module conformance under tinyjs (the target-side command mode)
  is M9 with the runner; the modules are verified under Bun (LocalHost)
  and against the store-backed VirtualHost here.
- The hostless → managed-host hand-over on `outside` is M11; today the
  tool result carries tinybash's refusal line, exit 2.
- The tinybash "Reference suites" (just-bash compat, oils spec, GNU tests
  filtered to the subset) from checkpoint 1 remain undone.
- `printf` is outside the tinybash subset (`echo -n` / `echo -e` cover
  most uses); worth a decision, since models reach for it.

`printf` joined the subset (user decision): bash's builtin for `%s %c %d
%i %u %x %X %o %%` with `-` `+` space flags, width and precision, format
reuse, bash's `invalid number` reporting, and the format escapes; `-v`,
`*`, `%N$`, `%b`, `%q`, floating conversions, `#`, `'`, `0` with strings
and `\u` are outside. Sixteen corpus cases against bash 5.2. Pitfall:
bash's printf does not stop at `\c` in the format (only `%b` does) — the
golden said so and the implementation followed.

### Review round on the M8 code (2026-09-02)

An owner-requested review of tinybash, the command loader and tinyjs
against their records. Findings that are being fixed on the branch, in
order: the namespace decision (below), the argv/byte boundary in the text
builtins, `printf %c`, redirect words, cancellation inside builtins, the
duplicated helpers, the `builtins/table.ts` import cycle, the leaked
timer in `HostlessEnvironment.exec`, the `strayImport` check in the
manifest build (deleted: a value import in a module fails at load time by
itself; a construction-time check hid nothing and invented a rule), and
the stale sentences in the records.

Outcome: all of it landed, one commit per group; what stays deferred is
under Open items. Kept apart on purpose: glob's and grep's bracket
translators — they share the class table now, but negation spelling
(`!` vs `^`), the outcome of an unknown class (no match vs refusal),
range validation and case folding differ, and one parameterised
translator would be harder to read than two short ones. Pitfall while
regenerating goldens: `ls -l` goldens record the Lima user, and the
regenerating machine's user differed from the original one's, so 308
goldens changed in nothing but the user name; those were restored from
git and only the 19 that the new `utf8.txt` fixture file affects
changed.

### Namespace decision made exact (2026-09-02)

The parse-first check simulated `cd` as always succeeding, so `cd
missing; cat ../outside.txt` was accepted and read above the home; it
also checked only a glob's literal prefix, so `cat */../../x` passed.
The owner's first question was whether the decision belonged at run
time instead; it cannot, because an upgrade re-runs the whole script on
a machine and anything already run hostless would run twice. The second
was whether the subset would have to shrink until the decision could be
made statically. It does not: the subset already has the property that
no string is computed at run time, so the filesystem is the only unknown,
and it enters only through `cd` success, glob matches and `mv`/`cp`
landing paths. Rather than argue it, a random-script test was written
first (`namespace-fuzz.test.ts`: generated scripts over `cd`, `..`,
globs, `$PWD`, variables, chains and the mutating builtins, executed
against a recording filesystem) and run against the old check; it
reproduced both holes and found three more — `find … || cd a; cat
a/b/../../..` (a `cd` in a conditional branch treated as certain), `cd a
|| X=…; > $X/../..` (a conditional assignment treated as certain; `$X`
empty resolves to `/`), and `mv … */..` (a glob in the last operand
shifting which operand is the destination). The check now carries a set
of candidate states, decides a `cd` only when it is unconditional and
nothing before it could reshape the tree, checks glob text at its own
depth, and checks every `mv`/`cp` operand as a source of the last. Ten
thousand rounds pass; the test stays in the suite.

Decisions: no run-time guard (the check is the guarantee; a guard would
hide a check bug rather than surface it); the hostless tree holds no
symbolic links (nothing hostless can create one; content that carries
one is an upgrade), which is what makes text-level resolution exact;
`parseTinybash` is async and takes the `fs` so an unconditional `cd src`
followed by `../x` stays hostless instead of going to a machine.

The same fuzz surfaced run-time crashes, since fixed with corpus cases
derived from bash 5.2: a directory redirected to stdin threw out of the
builtin (each tool now reports it its own way — `cat: -: Is a
directory`, `head: error reading 'standard input'`, `grep: (standard
input)`, `sort: read failed: -`, `sed: read error on stdin`, exit 4 …);
`>>` to a directory failed after the command instead of before it; a
glob expanding `test -d s*` into four words threw a parse-time refusal
at run time (now bash's `too many arguments` / `binary operator
expected`, exit 2; the same for any builtin whose whitelist a glob
expansion escapes, reported as the refusal line with exit 2). Redirect
words are now expanded, split and globbed with bash's `ambiguous
redirect` for anything but one word, naming the word as written.
`printf '%c' ''` prints a NUL. The suite is green: 377 cases including
the corpus, 4 Linux-only skips.

### M8 close-out and the M9 order (2026-09-02)

Owner decisions after the review round:

- **Still M8.** The tinyjs findings and tinyjs in command mode are M8
  leftovers, not M9 work: M9 starts on top of them, so M8 is not closed
  until they land. Close-out list: `kill` validated against the child
  table, `spawn` with `stdin: "null"` returning a null stdin handle,
  `import.meta.url` as a URL, the `/embedded/` prefix unreachable from a
  file-loaded module through the module cache, the packed section's
  `version`/`abi` check, and `tinyjsc`. Two findings stay open for a
  proposal first: tee writes on the loop thread, the TLS configuration
  rebuilt per request. tinyjs in command mode was first put on this list
  and then moved to M9 (owner decision, same day): it is `host-runner`
  plus a directory `ManifestSource`, module import by file path and an
  entry bundle — M9's first step, not a leftover.
- **`tinyjsc`.** The packer leaves the tinyjs binary: packing is always
  cross (one machine packs every target), so the runtime should carry no
  packing code, and one tool with one job is easier to keep right than a
  runtime with a `--pack` mode. `tinyjsc` is a second binary from the same
  crate, released with tinyjs; the bare binary is given explicitly
  (`--bin`), the release check is `tinyjsc`'s at pack time and tinyjs's
  at start (`tinyjs.md`, "Entry modes").
- **Naming by role.** The Host inside the runner is `@demicodes/host-runner`
  (it serves user hosts and managed hosts; "tinyjs" is the runtime it
  runs on, not its role). `@demicodes/host-virtual` stays the hostless
  Host. `RemoteHost` in `runner-protocol` is the backend's end of a runner
  connection. `overview.md` carries the table.
- **`host-local` is not in the final design** and is deleted in M9 with
  the Bun runner and the local open-box assembly (`repl`, `agent-eval`,
  `agent/server`, `web/agent-hub`). The backend's one use of it, a Node
  filesystem over its data directory, becomes `node:fs` and can move
  before M9.
- **M9 in dependency order**, lightest first: `host-runner` → the
  protocol at both ends → the runner port with the relay and the loader's
  runner side → output and media by reference → the `host-virtual`
  reduction → the deletions. `roadmap.md` carries the list.

### M8 close-out: tinyjs (2026-09-02) — M8 closed

Landed in `packages/tinyjs`, `cargo test` green (56/56 conformance, the
packing test, two unit tests) on macOS arm64; release build 2.4 MB for
each binary:

- The crate is a library plus two binaries. `tinyjsc` (`src/bin/tinyjsc.rs`
  over `src/pack.rs`) packs; `tinyjs` only reads the section. The section
  starts with a header (magic, abi, release) and every binary carries a
  `TINYJS-RELEASE:<release>:<abi>` marker as a `#[used]` static that the
  runtime parses for its own release, so no linker drops it. The marker
  prefix and the section magic are spelled backwards in the source and
  reversed at compile or run time, so their only forward copy in a binary
  is the marker itself, respectively an injected section: `tinyjsc` can
  search a bare binary's bytes for them without false hits from its own
  string constants. Refused at pack time: another release, an already
  packed binary, a file that is not tinyjs, a bundle importing a second
  file (the loader compiles against a payload with no modules beside the
  entry). Refused at start: a section from another release.
- Loader: `/embedded/*` is refused from a file-loaded module, checked
  after normalization so `../embedded/…` cannot reach it either;
  `import.meta.url` is a `file:` URL.
- `kill` accepts only pids in the child table; `0`, `-1` and the own pid
  fail with `ESRCH` before the syscall.
- `spawn`'s `stdin` was always `null` for `stdin: "null"`; the record said
  `fd`. The record now says `fd | null` — the code was right and the
  contract was misstated.
- The conformance test generates RSA certificates: the EC keys macOS's
  LibreSSL writes fail to decode in Bun's BoringSSL, which is why the
  stub-backed net cases failed on this machine at first.

Toolchain on a fresh machine: `brew install rustup`, then
`rustup default stable` with `/opt/homebrew/opt/rustup/bin` on the PATH;
Bun and openssl for the stub cases.

## M9 — Runner on tinyjs, old paths deleted (2026-09-02)

### Step 1: `host-runner` and command mode (2026-09-02) — delivered

Landed, in dependency order:

- `@demicodes/shell` root is the command system and the Host contract
  only; `BashEnvironment` and the portable command set moved under
  `@demicodes/shell/bash` (deleted in step 6 with just-bash), and
  `CommandRegistry` takes its reserved-name set from the engine instead of
  deriving it from just-bash. Nothing that runs on tinyjs may import the
  bash entry.
- The Host conformance suite: `hostConformanceCases` in
  `@demicodes/shell/testing`, runtime-neutral (contract + utils, no
  `bun:test`), 18 cases over spawn (stdio, stdin, kill, env isolation,
  cwd, the three spawn-error kinds, `openCwd`), fs (tree operations,
  links, metadata, errno codes, the artifact directory, one namespace
  shared with processes) and the store. `host-local` runs it under
  `bun:test` beside its dirfd-specific case; `host-runner` runs it on
  tinyjs.
- `fileHostStore(fs, root)` in `@demicodes/shell`: a `HostStore` as
  portable-JSON files on any `HostFileSystem`, temp-and-rename writes.
  `LocalHostStore` is gone; `LocalHost` and `host-runner` both use it.
  `errnoError` in `@demicodes/utils` builds the errno-coded errors a Host
  raises itself (`host-virtual`'s local copy removed).
- `@demicodes/host-runner`: `createRunnerHost` over `tinyjs:fs`,
  `tinyjs:process`, `tinyjs:runtime` (types declared once in
  `src/tinyjs.d.ts`); `rm -r`, `cp` and a cross-device `mv` composed from
  the primitives, file copies streamed in 1 MiB reads; spawn errors
  classified through errno with the cwd checked first; a validated
  logical cwd; the process's argv/env/stdio exported as typed access for
  the runner and the entry. `@demicodes/host-runner/testing` finds the
  binaries (building the crate when absent) and bundles an entry with
  Bun (`conditions: development`, `tinyjs:*` external). The conformance
  run is a Bun test that bundles `src/conformance/main.ts` and runs it on
  the bare binary, asserting `openHandles() === 0` at the end.
- The loader's runner side: `ManifestSource.modulePath(hash)`,
  `directorySource(dir, fs)` over `manifest.json` +
  `modules/<hash>.mjs` (the layout `writeManifestDirectory` produces),
  `importCommandModule(specifier)` in the shell and
  `CommandExecutionContext.loadModule` — the loader maps a module's text
  back to its hash and imports the file when the source names one, the
  `blob:` route otherwise. Proven under Bun in the loader's tests and on
  tinyjs by command mode.
- Command mode: `packages/runner/src/tinyjs/entry.ts` selects the mode by
  `argv[0]` (`demi-runner` → runner mode, refused until step 3; any other
  name → `command-mode.ts`: the Host, the directory source at
  `DEMI_COMMANDS_DIR` or `~/.demi/commands/current`, dispatch with the
  process stdio, SIGINT/SIGTERM as the abort signal). The test bundles the
  entry, packs it with `tinyjsc`, symlinks `demi`, `nope` and
  `demi-runner` to the packed file, writes the `demi` manifest as a
  directory and runs `demi file create` (stdin) and `demi file read`,
  `--help`, an `rpc` leaf without transport (exit 1), an unknown root
  (127) and runner mode (2). `demi file read` measured 33 ms end to end
  (process start, 530 KB bundle with zod, manifest parse, module import,
  the read) on macOS arm64.
- The boundary test in `packages/core` re-aligned with the registry: it
  had scanned `packages/tinyjs` for a `package.json` since M7 and its
  graph lacked `command-loader`, `tinybash`, the backend's provider and
  protocol edges; it now reads the root `workspaces` and carries
  `host-runner`, the `shell/bash` subpath and the runner's new edges.

Pitfalls:

- The shell root could not be bundled for tinyjs at all: `just-bash`
  reaches `@mongodb-js/zstd`, which `require`s a Node builtin, and Bun's
  bundler refuses that for a browser target regardless of tree shaking.
  The `bash` split is what makes the root bundleable; nothing else in the
  root imported Node.
- `openHandles()` caught a leak the conformance cases themselves passed:
  every child's stdin pipe handle stayed open unless the caller closed
  it (8 of 9 spawns). The Host now closes it when the child exits.
- `fileHostStore` saw keys with a trailing slash from
  `AgentSessionCommandStorage` (`agent-sessions/<id>/`) and produced
  `//` paths; keys are normalized.
- zod 4's `fromJSONSchema` and the loader's tree building run on QuickJS
  unchanged; the 530 KB bundle costs nothing visible next to process
  start.

Decisions: the conformance suite lives with the contract in
`@demicodes/shell/testing` and is the acceptance of every Host, so
`host-local` runs it too rather than keeping its own copy of the same
cases; the manifest directory layout (`manifest.json` + module files) is
the runner's cache layout from step 3 on, with `commands/current` the
symlink the runner maintains; the entry bundle belongs to
`@demicodes/runner` (the package whose dependency footprint it is).

### Step 2: the wire at both ends (2026-09-02) — delivered

Landed in `@demicodes/runner-protocol`, the Bun runner and the backend,
protocol version 2:

- Frames are binary MessagePack. The package no longer carries a codec of
  its own: `createRunnerWire(codec)` takes a `{ encode, decode }` pair —
  `msgpackCodec` over `@msgpack/msgpack` on the Bun ends, `tinyjs:bytes`
  on tinyjs (its `bytes.rs` already follows `@msgpack/msgpack`'s
  defaults). A test encodes every message shape on Bun, has tinyjs decode
  and re-encode the frames, and compares byte for byte; the tinyjs
  conformance case for the protocol bundle now frames over `tinyjs:bytes`.
- fs RPC is one message per operation: `fsOps` in `schemas.ts` is a table
  of `{ params, result }` schemas per `HostFileSystem` method, and the
  `fs_<op>` request union, the `fs_ok { id, op, result }` union and the
  `FsParams` / `FsResult` types all derive from it; `fs_error { id, code?,
  message }` carries the errno code. `RemoteHost` builds the request from
  the method call, `HostRpcServer` dispatches through a handler table keyed
  by op. A malformed parameter or a result of the wrong shape is refused
  at decode on either end.
- `pong { jobs }`: the Bun runner reports 0 (no job table until step 3);
  the registry keeps the count per connection (`runningJobs(deviceId)`)
  for the idle rule.
- `hello_error { code, reason }` with `unsupported_protocol`,
  `unknown_device`, `already_connected`, `revoked`, `internal`. The
  one-connection-per-token rule replaces the old "newer connection wins":
  a second hello on a connected token is refused with `already_connected`
  and logged (`RunnerRegistryOptions.log`); the runner retries that one
  code with its backoff and stops on every other. Covered by a backend
  test: the twin is refused while the first is online and is accepted
  after the first stops.
- The backend socket route sends and receives binary frames; a text frame
  is malformed and closes the socket.

Pitfalls: `registry.ts` carried two literal NUL characters in string
literals (a key separator and a sentinel), which made git treat the file
as binary and hid its diffs; they are `\0` escapes now. hono's Bun
adapter hands a binary message over as `message.buffer`; the frames decode
cleanly through the M4 suite, so the buffers are not pooled views.

### The view is the model's view; cwd carries, env does not (2026-09-02)

Two owner decisions before step 3, both written into `runner.md`,
`sessions-and-targets.md`, `commands.md`, `backend.md`, `overview.md`,
`product.md` and the roadmap:

- **No output beyond the model's view exists anywhere but on the target.**
  The records had carried a 1 MB "bounded view" per stream that crossed
  the wire and was stored, plus an `output_upload` message and an
  `/outputs/:ref` API for the browser to fetch full output by reference.
  The 1 MB was the inherited `DEFAULT_OUTPUT_LIMIT_BYTES`, not a designed
  number, and its only consumer would have been a browser view of output
  the model never saw — a requirement nobody had. Both are gone. The wire
  carries the model's window: the first bytes while the job runs (so a
  running command can be polled) and the last bytes of each stream at
  exit, read by the runner from the output file so the tail is the true
  tail. Today's `ShellStreamView` (1 MB, offset paging) survives only for
  its two programmatic consumers, the command bridge's `runCommandLine`
  and `demi host prev shell`'s tar pipe, and is deleted with them in
  step 6.
- **Shell state across jobs: the working directory carries, nothing
  else.** Surveyed: Claude Code, Codex CLI, Gemini CLI and Aider run a
  fresh process per call; only Claude Code carries the directory (tracked
  by the harness, reset when it leaves the project), none carries
  environment variables; OpenHands and Cline keep a persistent shell
  (tmux with a PS1 JSON marker, respectively the VS Code terminal). The
  runner records `pwd` from an `EXIT` trap and returns it in `job_exit`;
  `export` does not persist, matching the mainstream rather than inventing
  a half-persistent state.

### Step 3: the runner on tinyjs (2026-09-03) — delivered

Landed across `runner-protocol`, `host-runner`, `runner`, `backend` and
`tinyjs`:

- Protocol: `job_start` / `job_output` / `job_exit` (the view, the output
  paths, the tails, the final `cwd`), `job_stdin` / `job_stdin_end` /
  `job_kill`; `rpc_call` / `rpc_stdin` / `rpc_stdin_end` and `rpc_output`
  / `rpc_exit`; `manifest`. `JobTable` (runner side, runtime-neutral over
  an injected teed spawn) and `RemoteShellEnvironment` (backend side, the
  `ShellEnvironment` of a real host). `msgpackCodec` moved under
  `@demicodes/runner-protocol/msgpack`.
- host-runner: `spawnTeed`, `readTail`, the WebSocket and Unix-socket
  links, the codec re-export, `fdNode`. tinyjs: `read(fd, max, offset?)`
  (pread) and `runtime.fdNode(fd)`; 57 conformance cases.
- runner: `RunnerMode` (connect with backoff, hello/claim, the message
  loop, the state files, jobs, manifest install, relay), `RelayServer` and
  the command-mode relay client, `ManifestCache` with `current` and the
  root symlinks, the stdin classification in command mode;
  `@demicodes/runner/testing` packs the binary once per test process and
  starts `demi-runner` with its lines captured.
- backend: the shell environment is chosen per Host (`VirtualHost` →
  hostless, `RemoteHost` → jobs), the registry pushes the manifest on
  `hello_ok` and `claimed` and runs relayed rpc calls with the
  conversation's tree, storage and Host.
- Tests: M1 (bare AgentServer over a runner process, with the wire audit:
  a `printf | tee | cat` turn is `job_output` and `job_exit` frames and
  nothing else), M4 (pairing, restart, revoke, expiry, rate limit, the
  acceptance with a `demi todo add && demi todo list` turn over the
  relay, one connection per token), M6 (the switches, the tar pipe
  through `demi host prev shell` relayed) all on the tinyjs runner.

Pitfalls, each one a rule now:

- macOS ships bash 3.2: `exec {var}<&0` is not there, so the prelude uses
  a fixed descriptor (199) and the env names it.
- On macOS `stat("/dev/fd/0")` reports the devfs node, not the pipe, and
  the two ends of a pipe have different inodes: identifying the job's
  stdin from the runner's side fails. Two `fstat`s in the same process
  (`fdNode(0)` against the duplicated descriptor) is the OS-independent
  check.
- An explicit `undefined` for an optional argument of a tinyjs primitive
  fails conversion (`wsConnect(url, undefined)`): pass fewer arguments.
- A Unix socket path is at most 104 bytes on macOS; test state dirs must
  be short (the scratchpad path was not).
- A command-mode process could not find the relay: the job's env had no
  `DEMI_HOME`; the runner now sets it in every job.
- The relay client awaited its live-stdin forwarder after the exit, and a
  job's stdin never ends: the forwarder is abandoned with the socket.
- An in-process `Bun.build` for a browser target leaves the test process
  unable to resolve `@msgpack/msgpack` from `codec.ts` afterwards (only
  when the codec's package sits in the workspace package's own
  `node_modules`); the bundler runs as a subprocess now.
- A running command's `stdout.delta` was empty until exit: the record's
  text must grow with the streamed chunks, not only its chunk list.

### Step 4: transfers and media by reference (2026-09-03) — delivered

The plan as presented and accepted: the transfer's unit is a job's stdout
file; `demi host shell --id` runs the script as a job on the named host
and moves that file over HTTP; a caller on a device fetches it through
its own runner (`rpc_transfer`), a hostless caller takes the bytes
in-process; `transfer_receive` is the symmetric file end for M11's
hostless → managed placement; the browser gets transcript media as
`{ type: 'ref', ref, mediaType }` plus `GET /api/blobs/:sha256`.

Landed:

- Protocol (v3): `transfer_send`, `transfer_receive`, `rpc_transfer`
  (b → r) and `transfer_done` (r → b). `url` is origin-relative; the
  runner resolves it against its backend URL and authenticates with the
  device token.
- host-runner: `httpUploadFile`, `httpGet`, `writeStreamToFile` over
  `tinyjs:net.httpRequest` (file bodies stream; response bodies are
  handles).
- runner: `TransferClient`; runner mode answers the two transfer messages
  with `transfer_done`; the relay writes a call's replies through one
  per-call chain so an `rpc_transfer` body finishes before `rpc_exit`.
- backend: `TransferBroker` (single-use ids, source/destination device
  checks, the `PUT` held until the `GET` or the in-process consumer
  drained, timeouts, `deviceGone`), `/api/transfers/:id` routes, the
  registry's `transferSend` / `transferReceive` settled by
  `transfer_done` and failed on disconnect, the relayed rpc io carrying
  the calling device as a `transferDestination`; the `demi host` group
  gains `list` and `shell --id`, with `prev shell` on a machine routed
  through the same `runOnHost`; the reachable set is one function
  (`reachableHosts`) for M11's grant table to widen. `GET
  /api/blobs/:sha256`; the conversation-scoped transport externalizes
  media on every outbound transcript frame; the web UI resolves a `ref`
  source to the blob URL.
- Tests: `transfers.test.ts` (runner end against a fake backend: receive
  into a file, send a file, a refused exchange reported),
  `transfer-broker.test.ts` (runner-to-runner pipe, in-process consumer,
  timeout, wrong device, disconnect), `host-shell.test.ts` (two tinyjs
  runners, a 300 KB file copied by `tar` through `host shell --id`, the
  wire audit — the source socket carries `job_*` and the transfer
  control frames, the caller's socket carries under 1 KB of `rpc_output`
  — a refused id, and the hostless caller reading the copy), the
  attachments test asserting the `ref` form live and after restore and
  the blob route. A `trace` option on the registry records every message
  per device for such audits.

Pitfalls:

- A Bun server that answers a `PUT` without reading its body resets the
  connection, so the client sees `ECONNRESET` instead of the status. The
  broker refuses unknown ids without draining (a large body would be
  waste); the runner reports the reset as the failure, which is enough.
- The command parser takes a rest field only after a literal `--`; a
  script given as one quoted argument is a positional (`positionals:
  ['script']`).
- The wire audit's first cut asserted "no `rpc_output` frames" on the
  caller's socket and tripped over `demi host list`'s own lines; the
  assertion is now a byte count, and the trace carries whole messages.

### The final package structure (2026-09-03) — decided, records updated first

Before steps 5 and 6 the package set was re-drawn, on two review
findings from the owner:

- **`tinybash` depended on `shell` for six types.** A shell is
  infrastructure like tinyjs and owns its system interface: tinybash now
  declares `TinybashFs`, `TinybashIO`, `DispatchIO` and `RootPaths` in
  `src/host.ts` and depends on `utils` alone. The adaptation of Demi's
  Host contract and loader to it is `HostlessEnvironment`, moved from the
  backend to `@demicodes/shell/hostless` (the place `shell/bash` held for
  just-bash), taking `roots` and `dispatch` instead of a `Loader`;
  `shell → tinybash` is the one new edge, and only through that entry.
- **`host-runner` was misnamed.** The `host-` prefix means a Host the
  backend injects into the agent (`host-virtual`, and now `host-remote`
  for `RemoteHost` + `RemoteShellEnvironment`, out of `runner-protocol`).
  The Host over tinyjs is none of that: it is the runner's own access to
  its machine, held by nobody else. It is now `packages/runner/src/machine/`;
  `HostRpcServer` and `JobTable` are `runner/src/serve/`;
  `runner-protocol` is the wire alone, depended on by both ends.

Alongside: `LocalHost` becomes the Node Host under `@demicodes/shell/node`
(never a package again; tests run against it), beside `nodeFileSystem`,
the backing of the store-backed Host on the backend machine — the
`testing` entry stays runtime-neutral so the conformance suite bundles for
tinyjs; `AgentServer.shellEnvironment` is required;
the old local products (`repl`, `agent-eval`, the web package's server)
go with the Node Host they ran on rather than surviving to a rename in
M13; the design records that described them (`bash-behavior`,
`command-bridge`, `just-bash-fork-policy`, the binary-stream and
shell-yield plans, the REPL/eval/web/library plans under `docs/internal/`,
the Host and UI guides, the `examples/` directory) are deleted, not
annotated. `package-boundaries.md`, `overview.md`, `runner.md`,
`tinyjs.md`, `tinybash.md`, `commands.md`, `backend.md`, `roadmap.md`
(steps 5 and 6 rewritten to this plan) and `README.md` state the final
structure; the code follows in the next commits, one package per commit.

### Steps 5 and 6: the structure made final, the old paths deleted (2026-09-03) — delivered; M9 closed

Landed, in the order the records were written:

- `@demicodes/tinybash` depends on `utils` alone: `src/host.ts` declares
  `TinybashFs`, `TinybashStat`, `TinybashDirent`, `TinybashIO`,
  `TinybashWriter`, `DispatchIO`, `RootPaths`; the corpus fixtures import
  shell only as a dev dependency.
- `@demicodes/shell`: `hostless` (`HostlessEnvironment` over `roots` +
  `dispatch`), `node` (`nodeFileSystem`, `LocalHost` with `LocalHostCwd`),
  `testing` kept runtime-neutral (memory store, conformance suite),
  `reserved-names.ts` (one table: shell words, Unix tools, toolchains);
  `bash`, `host-fs`, the environment files, the portable commands and
  their ten test files deleted; the explicit offset paging
  (`ShellStatusInput` offsets, per-call `maxOutputBytes`, `ShellViewInput`)
  deleted — a status view is the delta since the last view, the record
  keeps the cursor.
- `@demicodes/command-loader/testing`: `hostlessShell` (the hostless shell
  over any Host with Bun's transpiler), `hostlessShellFactory`, and the
  `probe` root (`hold <ms>`, `stdin [--delay]`) that stands in for `sleep`
  and `read` in tests — a builtin never sleeps and never reads the
  script's stdin.
- `@demicodes/host-remote`: `RemoteHost`, `RemoteShellEnvironment`.
  `@demicodes/runner-protocol`: the wire only, plus the job env names
  (`JOB_CWD_FILE_VAR`, `JOB_STDIN_FD_VAR`, `JOB_STDIN_FD`).
  `@demicodes/runner`: `machine/` (the former host-runner), `serve/`
  (`HostRpcServer` with the device `PATH`/`HOME` fallback for spawns,
  `JobTable`, `device-env.ts`), `relay/`, the entry modes at the root;
  `testing.ts` carries `tinyjsBinary`, `bundleForTinyjs`, `packedRunner`,
  `startTinyjsRunner` and re-exports `HostRpcServer`; the Bun runner, its
  bin and `RunnerClient` deleted.
- `@demicodes/agent`: `shellEnvironment` required, `defaultShellEnvironment`
  and `runCommandLine` (with its error types and test) deleted;
  `ShellEnvironmentOptions` replaces the engine's option type everywhere.
- `@demicodes/host-virtual`: `process.spawn` throws; the refusal handle
  and `VIRTUAL_UPGRADE_GUIDANCE` deleted.
- Backend: `nodeFileSystem(dataDir)` backs the virtual hosts; the shell
  factory has no third branch; `demi host prev shell` on a hostless prev
  is refused with the reason (the switch places the files, M11); the
  claude-chain and llm tests run on the tinyjs runner.
- Deleted outright: `packages/just-bash` (submodule, `.gitmodules`,
  workspace entry, tsconfig paths, the `test:just-bash-core` script),
  `packages/host-local`, `packages/repl`, `packages/agent-eval`,
  `packages/web/src/server` with its three e2e tests (the web package is
  the Vite scaffold until M13), `examples/`, `packages/host-runner`; the
  changeset fixed group and the versioning record list the final set.
- Tests: every suite runs a shell through `hostlessShell` over
  `LocalHost` where it needs one; scripts moved into the tinybash subset
  (`printf`, `test -e`, assignments instead of `export`, `grep … > /dev/null`
  instead of `grep -q`, a grep-based check standing in for `bun test`);
  the todo isolation tests build one environment per agent session, as
  the product does. Green: agent 243, coding-agent + providers 136,
  backend 41, runner 10, shell + tinybash + command-loader +
  runner-protocol + host-remote + host-virtual + core + utils + web-ui +
  web 561; typecheck and the web typecheck clean.

Pitfalls, each a rule now:

- A busy default shell must not block the next `shell_exec`: both engines
  give the session a fresh shell when its default is running a command
  (the old engine did; the hostless one had not), which is what lets
  `demi agent list` run beside a running `demi agent spawn`.
- `@demicodes/shell/testing` is bundled for tinyjs by the conformance
  test: anything Node in it breaks the bundle, which is why `LocalHost`
  lives under `node`, not `testing`.
- A test fixture's store must be stable within the process but not across
  runs: `LocalHost` keys one temp store per working directory in a
  process-level map (a reopened session finds its children; a fresh
  process starts clean — the old default under `~/.local/share` leaked
  todo ids between runs).
- The tinyjs runner's `HostRpcServer` did not fill `PATH`/`HOME` into a
  spawn's env the way the Bun runner had; the Claude Code CLI then died
  before its MCP handshake. The fallback is one function, shared by
  spawns and jobs.
- tinybash has no `grep -q`, no `read`, no `sleep`, no `sh -c`, and no
  `!` in `test`; the scripts say so plainly now instead of pretending a
  machine is there.

### The symmetries (2026-09-03) — delivered

A review of what sat where after M9 closed, each item a place where two
things of one kind lived in two kinds of home:

- `HostlessEnvironment` moved from `shell/hostless` to `@demicodes/host-virtual`,
  beside `VirtualHost`, the way `RemoteShellEnvironment` sits beside
  `RemoteHost`: one execution target, its Host and its shell. `shell`
  depends on `utils` alone again; `host-virtual → shell, tinybash, utils`.
- The test fixtures moved with it: `hostlessShell`, `hostlessShellFactory`,
  the `probe` root and `LocalHost` under `@demicodes/host-virtual/testing`;
  `nodeFileSystem` under `@demicodes/host-virtual/node`. `shell/node` and
  `command-loader/testing` are gone; the boundary test skips `testing`
  entries when it builds the production graph, and the host-injection
  rule for platform-neutral packages reads production dependencies only.
- The job environment names (`JOB_CWD_FILE_VAR`, `JOB_STDIN_FD_VAR`,
  `JOB_STDIN_FD`) returned to the runner's `serve/jobs.ts`: both users are
  runner-internal. `HostRpcServer` and `JobTable` are reachable at
  `@demicodes/runner/serve` for host-remote's tests; `runner/testing`
  re-exports nothing of the runtime.
- `Host.process.spawn` is optional: `VirtualHost` declares none, the
  conformance suite's process cases apply only where `spawn` exists, the
  backend refuses a process-capable provider on a hostless conversation
  with the reason, and a read-only subagent Host denies every write.
- No artifacts anywhere: `commandArtifactsDir` left the Host contract and
  every implementation, `CommandArtifactStore` and the meta/bin files
  are gone, the hostless environment keeps nothing beyond the view
  (`commands.md`'s table was right; the code now agrees). The status
  view's `outputDir` and stream `path` are present only when the target
  keeps output files (the runner's tee); the tool text prints paths only
  then, and a binary stream says "not kept beyond this view" hostless.
- `audit` deleted end to end: `BashAuditEvent`, the record and status
  fields, the `audit` frame, `progressToAudit`, the tool view field, the
  rendering-spec bullet.
- `prepareShell` and `heredocDelimiter` deleted (no caller since the
  command bridge).
- `commandModulesAsText` moved to `@demicodes/command-loader/build`.
- Recorded, not changed: `RESERVED_COMMAND_NAMES` hand-lists the Unix
  tools tinybash implements; shell cannot import tinybash, so the two are
  kept in step by review.

Green: agent 243, coding-agent + providers 136, backend 41, runner 10,
the rest 516 + core/host-virtual 44; typecheck and the web typecheck
clean.

## M10 — Scenario suite (2026-09-03)

Status: delivered — `packages/backend/src/__tests__/scenarios/`: the
world, the scripted model, the driver, S1–S9 on both targets (26 tests)
and R1–R4; the M2 detach case and the M3 restart case moved in from
`backend.test.ts`. `bun test packages/backend`: 69 pass.

Findings on the first run — composition defects none of the per-layer
suites could see, each fixed where it belonged:

1. `demi agent` was unknown on a real host: the manifest a runner caches
   was built from the backend's static tree, and the `agent` node is
   grafted per session. The manifest now carries the node's shape
   (`subagentCommandShape`, the harness's profile names), and a relayed
   `rpc` call runs against the registry the session's shell was built with
   (`sessionShells` in the backend), not a re-derived tree.
2. A relayed `rpc` call from a subagent resolved its Host by conversation
   id and landed on a fresh virtual host; the same lookup made every child
   run hostless while its parent was on a runner. The supervisor now
   resolves a child's Host as the root session (`ChildSupervisorOptions.
   agentSessionId`; `AgentHostContext.agentSessionId` is documented as the
   root's), and the relay uses the session's own shell context.
3. The read-only child Host (a plain-object wrapper) broke the backend's
   `instanceof` dispatch on both targets, and on a machine it policed only
   `rpc` leaves — bash and `runtime` commands write the disk. The owner
   removed the capability: no `readonly` profile flag, no Host wrapper;
   `explore` is a prompt and nothing more.
4. On a runner the model was told `stdoutBytes` equal to the view (head
   and tail), not the stream: the record now carries the stream's length
   beside its view text (`stdoutBytes`/`stderrBytes` on the record).
5. On a runner a job killed by `shell_abort` settled as `exited 130`;
   hostless said `aborted`. The remote engine marks the exit that follows
   its own kill as `aborted`.
6. After a streamed head, the runner's exit rebuilt the record's chunks
   while the merged-output cursor still pointed past the old ones, so the
   model's final preview was a slice of the wrong stream. The remote engine
   appends what the exit adds after the streamed chunks; a rebuild (binary
   placeholder, coverage repair) resets the cursors.
7. Absent optional flags crossed the wire as `null` (`undefined` as nil in
   MessagePack) and reached a leaf as the string "null". The loader drops
   `undefined`-valued args before the transport (`withoutUndefined` in
   utils).

Also: the switch announcement still spoke of "the artifacts directory";
the `tool_call` block's comment still named command artifacts. Both
corrected.

Recorded, not changed (allowed differences in `scenarios.md`, for the
owner):

- No `demi` command reads the script's live stdin hostless, so
  `shell_write` reaches nothing there; tinybash hands root commands the
  pipe and the loader's stdin field reads that. On a machine any program
  reads its stdin.
- A machine's shell carries its cwd between jobs and nothing else;
  tinybash's default shell keeps its variables.

Owner decisions after the first run: the read-only subagent capability is
deleted (item 3); and tinybash's unredirected stdin now follows bash — a
builtin whose stdin is not redirected reads the script's own (the
executor's top-level channel), `head` stops reading at its count as GNU
head does (an input that never ends no longer holds it), and a root
command keeps the empty pipe with the live stream beside it. The stdin
scenario of S3 runs on both targets.

R2 verdict: closing the backend while a command runs aborts the turn — the
job is killed on the runner (`job_kill`, `job_exit`), the tool call settles
as an error, and the turn closes with an abort block; after the restart the
next turn runs and the model sees the aborted call's result first. Nothing
dangles; no code change was needed.

Pitfalls: a scripted turn must end with a `response` (the ledger observes
usage from it), and a script cut off by an abort carries none — the
ledger invariant counts answered requests. Reused tool-use ids collapse
in the driver's de-duplication; polls carry unique ids. A runner's
`shell_write` returns at once; the scenario polls after it.

Why a milestone: M0–M9 proved each contract once, in one test file per
milestone, with the model as an in-process script. What was never proved
is the composition — the text the model reads after a tool call ran on a
target — and the two restarts. The suite is also the place later
milestones add their end-to-end cases, so it has to exist before M11
stacks grants, provisioning and hibernation on the backend.

Decisions:

- The model stays a provider-event script. A wire-level fake upstream and
  pinned `claude`/`codex` binaries in CI were considered and set aside as
  not the risk right now; the real-CLI chain keeps its `skip` gate.
- The browser is out; its acceptance is M13's manual checklist.
- The suite's observation is what the model received, not frames. Frames,
  files and databases confirm the mechanism.
- The tests the suite subsumes move in rather than staying beside it: the
  M2 detach case and the M3 restart case leave `backend.test.ts`. Pairing,
  provider assembly and the M6 switch acceptance stay.
- Renumbering: the former M10–M14 (access model and managed hosts,
  multi-user, web UI, packaging, scaled deployment) became M11–M15.
  Forward-pointing references in the records and the two test comments
  were updated; the earlier log entries in this file keep the numbers they
  were written with.

Plan (each a commit): the world, the model queue and the driver with S1 on
both targets; S2–S9 in order; R1–R4 with R2's verdict recorded here; the
two moved tests deleted from `backend.test.ts`; the verification row.

## M11 — Access model and managed hosts (2026-09-03)

Status: delivered — six checkpoints, each a commit; the rulings and the
delivered entries follow the opener below.

What the code holds against the two records (`sessions-and-targets.md`,
`managed-hosts.md`) at the start of M11:

- `control.sqlite` has no `conversation_host_grants`, no
  `devices.kind` / owner columns, no `conversations.host_device_id`. The
  prev slot (`prev_target_json`, `PrevTarget`, `demi host prev shell` /
  `release`, the `announced` flag) is still the mechanism behind switching;
  the records replace it with the grant set and M6's roadmap entry says so.
- `demi host list` / `current` / `shell --id` exist (`managed/host-command.ts`)
  over the current target and the prev slot; `reachableHosts` is the one
  place the grant set plugs into, as the record intends.
- tinybash reports a script outside the subset as `{ kind: 'outside' }`
  and `HostlessEnvironment` turns that into exit 2 with the refusal text.
  That is M8's stand-in for the upgrade: M11 replaces the exit with the
  provision-bind-run path.
- The hostless files live as a real directory under
  `<dataDir>/virtual/<conversationId>` (`scopedFsBackend`), not as the
  `files` tree with blobs that the 2026-09-02 decision and `storage.md`
  describe. Raised as a topic under checkpoint 3, where the home image is
  built from them.
- The wire already carries what the lifecycle reads: `pong { jobs }`,
  `hello_error`; `RunnerRegistry.runningJobs` and `AgentServer.sessionPhase`
  give the idle rule its two inputs. Nothing exists for `sync` before a
  kill, for the home-image growth request, or for a token on the kernel
  command line.
- tinyjs spawns with `uid`/`gid` (the guest-user drop); it has no mount,
  reboot or `/proc/cmdline` path, which PID 1 needs.
- The scenario world pairs packed tinyjs runners as user devices with a
  workspace each; the fake provisioner reuses that runner as the "VM".

Checkpoints, lowest dependency first:

1. **Access model.** The schema made final in the init migration (no
   deployment exists to migrate): `devices.kind`, `owner_conversation_id`,
   `owner_workspace_id`; `conversations.host_device_id`;
   `conversation_host_grants`. Resolution order workspace → session-bound
   host → hostless. The prev slot deleted: switching grants the departed
   host, the announcement names it and points at `demi host shell --id`.
   `demi host list` / `current` / `shell --id` over the grant set, the
   grant check in `reachableHosts`; the devices API gains grant and revoke
   per conversation and hides managed devices. Tests: grant authz, the
   switch scenario's announcement, the M6 switch acceptance re-pointed.
2. **The provisioner seam and the lifecycle**, against a fake provisioner
   and a local runner: `ManagedHostProvisioner` (provision, wake,
   hibernate, destroy) and the home-image store seam; the managed device
   row pre-created with its token; a managed hello without a token refused
   and never pending a claim; one active VM per owner with concurrent
   wakes joined; the idle rule (no in-flight turn and `pong.jobs = 0` for
   the window; the hard cap), hibernate, wake on the next action needing
   the host, the periodic checkpoint with the liveness exemption, the
   crash-loop guard, the per-user host-count cap, destroy on archive or
   delete; the `managedHosts` config. Owner-scoped authz on every check.
3. **The session upgrade.** `HostlessEnvironment` hands an outside script
   to an injected upgrade hook instead of exiting; the backend builds the
   home from the hostless files, provisions, binds `host_device_id`, hands
   tinybash's cwd and variables to the first bash job, and runs the script
   whole there; `/tmp` placed; nothing enters the transcript. The
   split-equivalence test (deferred from M8) and an S10 scenario in the
   suite. Topic to rule on first: the hostless files' form.
4. **Cloud workspaces.** The Cloud device choice on workspace creation
   provisions a host owned by the workspace with an empty home; every
   conversation under it runs there; idle counts turns across them.
5. **The runner as PID 1.** The tinyjs primitives init needs (mount, the
   command line, reaping, network configuration, reboot) and the runner's
   init mode: token and backend URL from `/proc/cmdline`, the mounts, jobs
   spawned as `demi`, `SIGTERM`; the `sync` message and the home growth
   request on the wire. Verified in a Linux container under a PID
   namespace where possible; the rest in the smoke.
6. **Images and Firecracker.** The guest kernel and rootfs pipeline (the
   preinstalled toolchain, `demi` with sudo, tinyjs as `/demi-runner` and
   `/usr/bin/demi`), the image tools (`mke2fs -d`, shrink, grow), the
   Firecracker provisioner under jailer with the privileged helper, tap
   networking and the egress rules; the env-gated smoke on Linux with
   `/dev/kvm` recording cold-provision and wake latency.

### Checkpoint 1 rulings (2026-09-03)

- The schema is made final in the init migration; no second migration,
  since no deployment exists to carry forward.
- The switch announcement keeps its carrier: the `preamble` slot of the
  next `user` block, filled by the harness preamble hook. The "switched,
  not yet told" state moves from the prev slot to
  `conversations.pending_switch_json` (`{ from, to }`), cleared on
  injection. Appending a notice block to the transcript at switch time
  was considered and set aside: it needs a new block type, replay and
  compaction rules and a turn-external write API in the agent package,
  for the same model-side effect.
- Explicit grants accept user devices only (the devices page never shows
  managed hosts); a managed host enters a grant set only by the automatic
  grant on switching away from it.
- `demi host shell --id` on a granted host starts in that device's home;
  the announcement names the departed directory by absolute path.

### Checkpoint 1: the access model (2026-09-03) — delivered

Landed:

- `control.sqlite` final for M11: `devices.kind`, `owner_conversation_id`,
  `owner_workspace_id`; `conversations.host_device_id`,
  `pending_switch_json`; `conversation_host_grants`. `ControlService`:
  `createDevice` takes a kind and an owner, `listDevices` returns user
  devices only, `switchConversationTarget` moves both pointers, records the
  pending switch and grants the departed device in one compare-and-set,
  `clearPendingSwitch`, `grantHost` / `revokeHost` / `listHostGrants` /
  `isHostGranted`. The prev slot is gone with everything that read it.
- `ExecutionTarget` (`hostless` | `workspace` | `host`) and
  `resolveExecutionTarget` (`conversation/execution-target.ts`): the one
  reading of the row as a target, used by the backend's Host resolution,
  the switch, the announcement and `demi host`.
- The switch (`conversation/target-switch.ts`): `no_hostless_entrance` for
  a session-bound managed host (409 on the PATCH); a managed host switched
  to a workspace is granted like any departed device.
- The announcement reads the pending switch, names both targets and
  directories, and points at `demi host shell --id <departed>` with the
  departed directory by absolute path; cleared on injection.
- `demi host list` / `current` / `shell --id` over the current target plus
  the grant set (`reachableHosts` is the one check); a granted host's shell
  starts in its home, which `HostIdentity.homeDir` now carries on the wire
  (`hello.runner.identity`), from tinyjs's identity on a runner.
- `GET/POST /api/conversations/:id/grants`, `DELETE
  /api/conversations/:id/grants/:deviceId`; a grant takes the user's own
  `user` devices only (404 otherwise); `GET /api/devices` never lists a
  managed host.
- Tests: `switch.test.ts` rewritten on the grant model (the announcement
  once per switch, the departed device granted and reached, the grant API
  with the managed-host refusal and the revoke, the hostless entrance
  refused for a session-bound host, the compare-and-set); `host-shell` and
  S6 re-pointed from `prev shell` to `shell --id`. `bun test
  packages/backend`: 70 pass.

Pitfalls: `$?` is outside tinybash's subset, so a hostless test script
cannot echo an exit status — `|| echo refused` instead. The runner suite's
`jobs.test.ts` fails on this machine independently of the change: the
developer's `~/.bashrc` sources a file under an empty `HOME`.

### Checkpoint 2 rulings (2026-09-03)

- The provisioner seam owns the VM and nothing else: `provision(owner,
  homeDir, bootArgs)`, `wake`, `hibernate`, `checkpoint`, `destroy`, a
  death notification. The Firecracker implementation composes the image
  tools (`mke2fs -d`, shrink, grow) and the home-image store inside
  itself; the fake implementation runs a local tinyjs runner over
  `homeDir` and keeps the directory across hibernate and wake. The
  lifecycle never touches an image, so every flow runs on macOS through
  the fake. The untouched-skip needs a runner report and lands with the
  wire work of checkpoint 5.
- `hello.runner.managed` marks a runner booted as init; managed without a
  token is `hello_error unknown_device`, never a pairing code.
- Owner keys are `conversation:<id>` and `workspace:<id>`; the managed
  device row is created at provision (name `cloud`), its token minted
  fresh at every provision and wake (`rotateDeviceToken`).
- The idle rule reads `sessionPhase` and `pong.jobs`; the registry gains
  `pauseLiveness` / `resumeLiveness` for the checkpoint pause. Defaults,
  all in `managedHosts` config: idle 10 min, hard cap 24 h, checkpoint
  every 15 min, crash loop 3 deaths in 10 min, 10 managed hosts per user.
- `BackendOptions.managedHosts` is optional; without it the upgrade hook
  reports the missing machine as an ordinary tool error.

### Checkpoint 2: the provisioner seam and the lifecycle (2026-09-03) — delivered

Landed, in `packages/backend/src/managed/`:

- `provisioner.ts`: `ManagedHostProvisioner` — `provision(owner, homeDir,
  boot)`, `wake`, `hibernate`, `checkpoint`, `destroy`, `onDeath`; `BootArgs`
  is the backend URL and the token. The seam owns the VM and nothing else.
- `lifecycle.ts`: `ManagedHosts` — `provision` (the device row `kind:
  managed`, name `cloud`, the owner column, the token minted and hashed,
  the per-user cap), `ensureRunning` (a running guest returns, a boot in
  flight is joined, a save in flight is awaited and then the guest woken
  with a fresh token via `rotateDeviceToken`, the crash-loop guard),
  `hibernate`, `destroy`, and the sweep: idle window over `turnInFlight`
  and `pong.jobs`, the hard cap, the checkpoint clock under
  `pauseLiveness`/`resumeLiveness`. `DEFAULT_MANAGED_HOSTS_CONFIG` holds
  the ruled sizes plus `bootTimeoutMs` and `sweepMs`.
- The backend: `BackendOptions.managedHosts` (provisioner and config),
  `Backend.managedHosts`; `hostFor` checks the owner of a managed device
  against the conversation or workspace and wakes it — the "next action
  needing the host"; `demi host shell --id` wakes a granted managed host;
  archiving a conversation and deleting a workspace destroy the guest.
- The wire: `hello.runner.managed`; the registry refuses a managed hello
  without a token (`unknown_device`), never issuing a pairing code.
  `whenOnline(deviceId)`, `pauseLiveness`, `resumeLiveness`; the manifest
  is now built before the device is bound, so the `manifest` frame follows
  `hello_ok` at once and no job can precede it (found by S10: the first
  job after a boot ran before the manifest and `demi` was not found).
- Runner: `RunnerModeOptions.managed` (`DEMI_RUNNER_MANAGED` until the init
  path of checkpoint 5 sets it from the command line); the test launcher
  takes a pre-issued `deviceToken` and `managed`, and exposes `exited`.
- Control: `getManagedDevice(owner)`, `countManagedDevices`,
  `rotateDeviceToken`, `listConversationIdsInWorkspace`.
- Tests: `scenarios/fake-provisioner.ts` and S10
  (`s10-managed-lifecycle.test.ts`) over the world with `managedHosts` and
  `pingIntervalMs`. `bun test packages/backend`: 75 pass.

Pitfalls: the liveness ping closes a connection whose previous pong is
still pending, so a test ping interval must exceed the time a debug
runner spends installing the manifest (the loop handles messages in
order) — 500 ms in S10. A hibernate's `calls` entry precedes the process
stop; the scenario waits for both.

### Checkpoint 3 rulings (2026-09-03)

- The hostless files take the form `storage.md` specifies: the `files`
  tree table in the conversation database with contents in the blob
  store, replacing the per-conversation directory under
  `<dataDir>/virtual/`. Ruled on the deployment ground: the N>1 topology
  replicates the conversation databases and the blob store, and a
  directory beside them is the one piece of conversation data outside
  that path. Done as the first step of the checkpoint, since the upgrade
  is where the tree becomes a directory.
- The upgrade lives in one backend wrapper over the two shell
  environments: `parseTinybash` decides before anything runs; an outside
  script materialises the tree, provisions, binds with
  `bindConversationHost` (the pointer alone, no pending switch, so nothing
  is announced), hands tinybash's cwd and variables to the first job as a
  prefix, and runs the script whole on the machine. A provisioning
  failure is that call's tool error; nothing is bound.
- `/tmp` is materialised under the home's `.tmp/`; the guest mounts it in
  checkpoint 5. The fake provisioner's `/tmp` is the test machine's own —
  an allowed difference.
- S11, split equivalence: the same script run whole on the machine and
  split at every point, tool results and final files compared byte for
  byte, over commands whose output is the same under BSD and GNU
  coreutils; the GNU-faithful corpus stays in tinybash's Linux job.

### Checkpoint 3: the session upgrade (2026-09-03) — delivered

Landed:

- The hostless filesystem is the `files` tree: `files` table in the
  conversation database (path, parent, kind, mode, mtime, size, sha256),
  bytes in the blob store; `storage/files-tree.ts` implements the
  `VirtualFsBackend` over it (no symlinks or hard links — EPERM),
  `materializeFilesTree` writes placements (`/home/demi` → the home,
  `/tmp` → its `.tmp/`) with modes and mtimes, `clearFilesTree` empties
  the rows after the upgrade. `ConversationStores.filesBackend` /
  `materializeFiles` / `clearFiles`. The `<dataDir>/virtual/` directory
  is gone; `scopedFsBackend` moved to `@demicodes/host-virtual/testing`.
- `HostlessEnvironment.outside(input)` — the parse-first decision under
  the state of the shell the exec would use — and `handoverOf(input)` —
  the cwd and the session's own variables.
- `conversation/upgrading-shell.ts`: `UpgradingShell` over the hostless
  environment and an upgrade callback; the first outside script provisions
  through the lifecycle, binds with `bindConversationHost` (the pointer
  alone, no pending switch), clears the tree, and runs on the machine's
  shell with `cd <cwd> && K=v` prefixed once — the hostless home mapped to
  the machine's home (`HostIdentity.homeDir`; `/home/demi` on a real
  guest, the staging directory under the fake). The machine's shell
  environment is the same instance the session uses afterwards: the
  backend keeps one per (session, Host).
- `bindConversationHost` on the `ControlService`; `Backend` materialises
  under `<dataDir>/staging/<conversationId>` and hands the directory to
  the provisioner, which owns it from then on.
- Without `managedHosts` configured, an outside script is the tool error
  "this script needs a machine, and this backend provisions none"; the M8
  refusal text is gone.
- Tests: S11 (`s11-upgrade.test.ts`) — the upgrade with files, `/tmp`,
  cwd and a variable carried, silence checked on the request items, the
  transcript and the record; a refusing provisioner; split equivalence
  over a seven-step sequence at every split point. `driver.readFile` and
  `world.hostlessFile` replace the hostless `filePath`. `bun test
  packages/backend`: 78 pass; host-virtual 22 pass.

Pitfalls: the split run makes ~130 provider requests in well under a
minute, past the default 120/min limit — `WorldOptions.
providerRequestsPerMinute`. On the fake the machine's home is not
`/home/demi`, so `$HOME` and absolute `/tmp` paths differ from hostless
there; the sequence uses relative paths and the record notes the
difference as the fake's.

### Checkpoint 4: Cloud workspaces (2026-09-03) — rulings

The topic: the Cloud device choice on workspace creation. Rulings:

- 4a. The request body: `{ deviceId, path, name }` for a workspace on a
  user device, `{ cloud: true, name }` for a Cloud one — a union, no
  device-type field.
- 4b. A Cloud workspace's directory is the host's home as the guest
  reports it in its hello (`/home/demi` on a real guest, the staging
  directory under the fake); the user picks no path.
- Once per project: `provision` is keyed by owner, so the workspace's
  host is one. A host that fails to come up is the request's error
  (409 with the lifecycle's code) and no workspace row is written.

### Checkpoint 4: Cloud workspaces (2026-09-03) — delivered

Landed:

- `managed/cloud-workspace.ts`: `createCloudWorkspace(deps, userId,
  name)` — the workspace id chosen first, an empty home under
  `<dataDir>/staging/<workspaceId>`, `managedHosts.provision` with the
  workspace as owner, the workspace row at the home the registry holds
  from the hello. A boot that fails destroys the guest and deletes the
  device row, since nothing references it yet, so a retry starts clean
  and the per-user count is not consumed.
- `POST /api/workspaces` takes the union body; `ManagedHostError` maps
  to 409 with its code; a backend without `managedHosts` answers
  `no_cloud` 409. `createWorkspace` takes an optional `id`.
- Nothing else was needed: the owner check and wake in `hostFor`, the
  idle rule over the workspace's conversations, and destroy on delete
  were checkpoint 2's.
- Tests: S12 (`s12-cloud-workspace.test.ts`). `bun test packages/
  backend`: 82 pass.

### Checkpoint 5: the runner as PID 1 (2026-09-03) — rulings

The topic: the runner's init mode on a managed guest, and the three
protocol additions around the home. Rulings:

- 5a. Init mode is chosen by `pid === 1`, no flag. Kernel command-line
  keys: `demi.backend`, `demi.token`, `demi.ip`, `demi.gw`, `demi.dns`.
- 5b. The runner's state directory is `/var/lib/demi` on the ephemeral
  upper, never in the home, so the runner's own bookkeeping cannot make
  the home "touched"; the command cache is rebuilt on every wake. The
  relay socket is mode 0666 in init mode (single-tenant VM; the guest
  user's command-mode processes must reach it).
- 5c. The upper is an overlay over the whole `/`, made the root by
  `pivot_root` from the rootfs, as the record says; the alternative of
  overlaying only `/usr`, `/etc`, `/var`, `/opt` was not taken.
- 5d. Messages: `sync { id }` → `sync_done { id, untouched }` before the
  kill; `home_grow { bytes }` → `home_grown { bytes }` for growth. The
  provisioner seam gains `growHome(owner, bytes)`; `hibernate` takes
  `{ untouched }`. Untouched is the block layer's sectors-written count in
  `/proc/diskstats` against a baseline taken after the mount.
- 5e. Verification: the init plan, the command line and the home image
  over recorded commands in Bun; `sync`/`sync_done` and the growth
  handshake in S10; the real mounts, the pivot and `resize2fs` in the
  checkpoint 6 smoke on Linux.

### Checkpoint 5: the runner as PID 1 (2026-09-03) — delivered

Landed:

- No tinyjs change: the record already had reaping as the only PID 1
  primitive behaviour and mount/network as spawns; `spawn` had uid/gid.
- `packages/runner/src/init/`: `cmdline.ts` (the command line parsed,
  `guestBootConfig`), `plan.ts` (`initPlan`: kernel filesystems, the
  upper assembled under `/run/newroot` with the kernel filesystems moved
  in and `pivot_root`, the home at `/home` and a tmpfs `/tmp`, `ip` for
  the network; `runInit` with tolerated steps), `home-image.ts`
  (`BlockHomeImage`: baseline and `sync` over diskstats, `wanted` over
  `df` with the reserve rule — a tenth or 256 MB, asking for double —
  `grown` running `resize2fs`; `DirectoryHome` for everything else),
  `boot.ts` (`bootGuest` binding the plan to the machine layer, the guest
  user constants, `/var/lib/demi`).
- `entry.ts`: `pid === 1` → `initMain`: boot, then `RunnerMode` with the
  token in memory, `managed`, the guest identity and `runAs` for every
  job and spawn, the block home image, `SIGTERM` → stop.
- `RunnerMode` options `deviceToken`, `guest`, `home`, `homeCheckMs`;
  handles `sync` and `home_grown`; asks `home_grow` after a job exit or
  on the minute check, one request in flight. The machine layer's host,
  process and teed spawn take `runAs`/uid/gid; the relay socket mode is
  an option. Protocol version 4; runner 0.21.0.
- Backend: `RunnerRegistry.sync(deviceId, timeoutMs)` (offline or timeout
  ⇒ touched) and the `home_grow` handler over a `homeGrow` option;
  `ManagedHosts.hibernate` syncs first and hands `{ untouched }` to the
  provisioner; `ManagedHosts.growHome`; `syncTimeoutMs` in the config.
- Tests: `packages/runner/src/__tests__/init.test.ts` (6); S10 checks the
  `sync`/`sync_done` frames and the provisioner's report, and a WebSocket
  standing in for a runner exercises `home_grow`/`home_grown` against the
  fake. `bun test packages/backend`: 83 pass; runner 16 pass plus the
  environmental `jobs.test.ts` failure noted under checkpoint 2.

Pitfall: the sync id must not come from the pairing-code generator; it is
a plain `createId()`.

### Checkpoint 6: images and the Firecracker provisioner (2026-09-03) — rulings

The topic: everything that lets the backend start a real guest. Rulings:

- 6a. Delivered in three commits under one checkpoint: (i) the home-image
  store, the image tools and the guest-image pipeline; (ii) the
  Firecracker provisioner, the launch modes, the install script; (iii) the
  env-gated smoke in the Lima `fc` instance with cold-provision and wake
  latency recorded here.
- **The jailer is a first-class option, not a requirement** (the user:
  "做成开关。并且是第一公民支持。不是必选的"). Two launch modes,
  `direct` (no root at runtime; KVM + Firecracker seccomp) and `jailer`
  (a privileged helper starts the jailer; per-VM uid, chroot,
  namespaces, cgroup), differing only in how the VMM process is started.
  The record's Provisioning and Security baseline sections say so now.
  Reasoning kept here: root is needed only by the jailer (chroot,
  namespaces, cgroup, setuid) and by tap/nftables setup; the latter moves
  to a one-time install script with a tap pool, so `direct` mode needs no
  root at all. For public service the jailer is what keeps a VMM escape
  inside one tenant; for self-hosting it is not needed.
- 6b. The helper is a small Rust crate (`packages/fc-helper`), static,
  two verbs, whitelisted arguments, one sudoers line.
- 6c. Egress: the address the runner is told to dial is the one explicit
  allow; private and link-local ranges denied; the rest allowed; no
  inbound. Installed once for the tap pool.
- 6d. The pipeline lives in `packages/guest-image/` (scripts and a kernel
  config, not a TS package); `vmlinux` and `rootfs.ext4` are release
  artifacts. The smoke may point at an existing kernel through the
  environment; the pipeline builds one.
- 6e. Production configuration by `DEMI_MANAGED_*` environment variables
  (`LAUNCH`, `FIRECRACKER`, `JAILER`, `HELPER`, `KERNEL`, `ROOTFS`,
  `VCPUS`, `MEM_MIB`, `HOME_MIB`, `SUBNET`); none set ⇒ no managed hosts.
- 6f. The smoke is `real-firecracker.e2e.test.ts` under
  `DEMI_FIRECRACKER_E2E=1`, run inside Lima `fc` (Bun and a Linux aarch64
  tinyjs to be installed there).
- 6g. Spawning `firecracker`, the jailer, `mke2fs`, `e2fsck`, `resize2fs`
  is the provisioner's transport — the intentional external-process
  exception — and `docs/package-boundaries.md` says so.

### Checkpoint 6 (i): the home-image store, the image tools, the pipeline (2026-09-03) — delivered

Landed:

- `storage/home-image-store.ts`: `HomeImageStore` (`has`, `put` — the
  file renamed into place, atomically replacing —, `get` — a working copy,
  reflinked where the filesystem offers it —, `delete`);
  `DirHomeImageStore` at `<dataDir>/homes/<ownerKey>.ext4`.
- `managed/firecracker/image-tools.ts`: `makeHomeImage(homeDir, image,
  nominalBytes)` (the directory becomes `/demi` in the image, consumed),
  `shrinkImage` (`e2fsck -fy` accepting 0 and 1, `resize2fs -M`, the
  report parsed, `truncate`), `growImage` (the file only; the guest grows
  the filesystem), `missingImageTools`, `runTool`.
- The guest init grows the filesystem into its file at every boot
  (`resize2fs /dev/vdb`, tolerated, right after the mount) and chowns the
  home recursively on the first boot only (`demi.firstboot=1`), since
  `mke2fs -d` keeps the backend user's ownership.
- `packages/guest-image/`: `runner/build.sh` (musl tinyjs with
  `guest-roots`, the bundle, `tinyjsc`), `kernel/build.sh` (Linux 6.1 on
  Firecracker's microvm config plus `extra.config`), `rootfs/build.sh`
  (debootstrap noble, `packages.txt`, the `demi` user with sudo, Bun/uv/
  rustup in its home, the runner at `/demi-runner` and `/usr/bin/demi`,
  `mke2fs -d`, shrunk). Scripts only; not a workspace package.
- Tests: `home-image.test.ts` — the store and the parser everywhere, the
  e2fsprogs round trip (make, `debugfs` reads the file back, shrink below
  nominal, grow back, `e2fsck -fn` clean) where the tools exist. Run in
  the Lima `fc` instance (Bun 1.4.0 installed there; the tree rsynced to
  `~/demi` since the home mount is read-only): 3 pass. macOS: 2 pass, 1
  skip. `bun test packages/backend`: 85 pass, 1 skip.

### Checkpoint 6 (ii): the Firecracker provisioner, the launch modes, the install script (2026-09-03) — delivered

Landed:

- `managed/firecracker/`: `config.ts` (`FirecrackerConfig`,
  `firecrackerConfigFromEnv` over `DEMI_MANAGED_*`, defaults: 2 vCPU,
  2 GB, 1 GB nominal home, `172.16.0.0/16`, 256 slots, Cloudflare and
  Google DNS), `slots.ts` (`SlotPool`: a /30 per slot, host `.1`, guest
  `.2`, tap `demi<n>`, a locally administered MAC), `boot-args.ts` (the
  kernel command line: `console=ttyS0 reboot=k panic=1 pci=off
  init=/demi-runner` plus the `demi.*` parameters), `api.ts`
  (`FirecrackerApi` over the unix socket with Bun's `fetch({ unix })`:
  ready-wait, configure, start, pause, resume, drive rescan), `vm.ts`
  (`startVm` in both modes; the process's exit is the VM's death in both:
  in `jailer` mode the helper stays as the parent), `provisioner.ts`
  (`FirecrackerProvisioner`: provision = make, shrink, store, boot with
  `firstboot`; wake = store copy, enlarge, boot; hibernate = kill, shrink,
  store — or discard when untouched; checkpoint = pause, reflink copy,
  resume, shrink the copy, store; growHome = enlarge the file, rescan;
  destroy = kill and drop the working image, the store keeps its copy).
- `BackendOptions.publicUrl` (guests dial it; `DEMI_BACKEND_PUBLIC_URL`);
  `main.ts` builds the provisioner from the environment.
- `packages/backend/scripts/install-managed-hosts.sh`: the tap pool, IP
  forwarding, NAT, the `inet demi` nftables table (established, the
  backend address, private ranges dropped, the rest accepted; input from
  taps only to the backend port and DNS).
- `packages/fc-helper`: the Rust helper, `vm start` / `vm kill`, built
  with `cargo build --release` (compiles on macOS too, runs on Linux).
- Tests: `firecracker.test.ts` (slots, boot args, configuration: 3).
  `bun test packages/backend`: 88 pass, 1 skip. The VM path itself is
  6 (iii)'s smoke.

### Checkpoint 6 (iii): the Firecracker smoke in both launch modes (2026-09-03) — delivered

`real-firecracker.e2e.test.ts` (gated by `DEMI_FIRECRACKER_E2E=1`), run
in the Lima `fc` instance (Ubuntu 24.04 aarch64, 4 vCPU, 8 GB, nested
KVM; Firecracker v1.16.1; the CI kernel 6.1.155 at `/opt/fc/vmlinux`;
the rootfs from `packages/guest-image/rootfs/build.sh`; the runner from
`runner/build.sh` on macOS by `cargo zigbuild`). One world over the real
provisioner: a hostless conversation upgraded by its first outside
script, `sudo` writing into the upper and `/` seen as an overlay, idle →
`sync` → kill → shrink → store, wake over the same home with the upper
gone and the filesystem grown back, 50 MB written past the reserve →
`home_grow` → `home_grown` → `resize2fs`, archive → destroy with the
image kept. Passes in `direct` mode and in `jailer` mode (the helper at
`/usr/local/bin/demi-fc-helper`, the pool re-created by the install
script with `--mode jailer`). Measured (nested virtualization; the guest
boot itself is about 5 s of each):

| | direct | jailer |
|---|---|---|
| cold provision + first job | 8.3–13.7 s (the high end with a cold page cache after the rootfs build) | 8.4 s |
| wake + job | 8.7–13.5 s | 13.3 s |
| home stored after hibernate | 8.5 MB (64 MB nominal) | same |
| home after growth | 64 MB → 128 MB file, 109 MB filesystem | same |

What the smoke found and what changed for it:

- A `bun test <file>` scans the whole tree for test files; the
  debootstrap work tree under `out/` carries Debian's symlink loops
  (`usr/bin/X11 -> .`) and the scan never returns — Bun was OOM-killed
  at 7.5 GB. The rootfs build removes its work tree.
- The API socket path must fit a unix address (108 bytes): VM ids are
  `vm-<12 chars>`, not the owner key.
- `/dev/kvm`: the install script adds the backend user to `kvm`.
- `mount --move /run` into a root that lives under `/run` is `ELOOP`;
  the old `/run` stays with the old root and the new root gets a fresh
  tmpfs after the pivot. PID 1's stderr is flushed before it exits, or
  the console never shows why init died.
- The hello reported the process's identity (root, `/`) instead of the
  guest user's: the backend ran the first job in `/`. `RunnerMode`
  reports `host.identity`, which init overrides.
- The job's `cwd` file is written by the job as the guest user into a
  directory the runner made as root: the job directory is 1777 in init
  mode.
- A killed guest leaves a half-open socket the ping would take an
  interval to notice, and the wake found the dead connection "online":
  `RunnerRegistry.disconnect` on hibernate, death and destroy.
- The growth reserve of 256 MB can never be met by a 64 MB home: the
  reserve is a tenth, raised toward 256 MB but capped at a quarter.
- The overlay copy kept the build host's uid on `/etc/sudoers.d` (sudo
  refuses it) and the guest had no hostname (sudo warns): `cp
  --no-preserve=ownership`, `/etc/hosts` in the overlay, `hostname demi`
  in the plan. `/dev` is the kernel's (`CONFIG_DEVTMPFS_MOUNT`); the
  plan no longer mounts it.
- The jailer already passes `--id`; with `--new-pid-ns` it forks and its
  parent exits, so the helper follows `firecracker.pid`; it remakes the
  jail's `root` and `run` as the VM uid with mode 0700, so the helper
  opens them and the socket to the backend's group once Firecracker
  listens.
- `FirecrackerProvisioner.running` after `destroy` (entry deleted) read
  as true.

The rootfs rebuilt by the corrected pipeline (ownership, `/etc/hosts`,
the work tree removed) passes the same smoke with a clean console.

## M12 — Multi-user systems (2026-09-03)

Status: delivered — four checkpoints, each a commit: (1) identity and
sessions, (2) the admin surface, (3) instance-mode enforcement, (4) the
tenant-isolation matrix and the frozen API table.

What the code holds against `product.md` at the start of M12:

- `control.sqlite` has `users`, `web_sessions` and `settings`, all unused
  beyond `ensureUser(STUB_USER)`: a constant master named `local` that
  seven route modules and the composition root read as "the current
  user". No cookie is read or written anywhere; `/api/auth/*` answers
  with the constant.
- Connections have no owner in practice: `vault.list()`,
  `assembly.catalog()` and `providerFor()` are instance-wide, and the
  subscription login flow is built with the constant as owner.
- Every test request (29 `fetch` sites, 12 stream sockets) is anonymous.

### Rulings (2026-09-03)

- **The instance mode is a startup configuration, not a setting.**
  `DEMI_INSTANCE_MODE=shared|isolated` is required; `createBackend`
  takes it as a required option; there is no endpoint that changes it,
  and the `settings` table goes (nothing is left to store in it).
  `GET /api/settings` stays as a read-only `{ mode }` for the page. A
  restart under the other mode with connections in the table refuses to
  start: the rows' ownership would contradict the mode.
- **User data is isolated absolutely.** A conversation, device,
  workspace, attachment or usage row is visible to its owner only; master
  and admin manage accounts and, in shared mode, providers — they never
  read another user's data. Another user's object answers 404, a role
  short of the action 403, no session 401.
- **The admin action in shared mode is "configuring providers"**: the
  record and the progress log say it that way; `/api/connections` keeps
  its M5 shape.
- Initial setup is `POST /api/setup { username, password }`, accepted
  while the instance has no users and 404 afterwards; `GET /api/setup`
  says whether it is needed. Setup logs the master in.
- Cookie `demi_session`: `HttpOnly; SameSite=Lax; Path=/`, `Secure` when
  the request arrived over https (directly or by `X-Forwarded-Proto`);
  30 days sliding, renewed when a request finds under 15 days left;
  logout deletes the row. Passwords hash with argon2id (`Bun.password`).
- A user changes their own password: `PUT /api/auth/password { current,
  next }`.
- Login lockout: five failures on a username lock it for a minute.
- Usage: a user sees their own ledger; in shared mode admins also get
  the instance's ledger grouped by user.

### Checkpoint 1: identity and sessions (2026-09-03) — delivered

- `auth/`: `User`/`Role` and `outranks` (a role acts on strictly lower
  roles), argon2id through `Bun.password`, `WebSessions` (256-bit token
  in the cookie, SHA-256 in `web_sessions`, 30 days sliding, renewed
  under 15 days left, an injectable clock), `LoginLimiter` (five
  failures lock a username for a minute).
- `http/`: the gate `authenticate` over `/api/*` exempting `/api/setup`,
  `/api/auth/login`, `/api/runner` and `/api/transfers`; the cookie
  helpers (`Secure` over https or `X-Forwarded-Proto`); `setup.ts`
  (`GET` `{ needed }`, `POST` creates the master atomically while
  `users` is empty and signs it in, 404 `already_set_up` afterwards);
  `auth.ts` (login with 401 `invalid_credentials` / 429
  `too_many_attempts`, logout, me, `PUT /password`). Every route module
  reads `c.get('user')`; conversations, devices, workspaces and the
  stream answer another user's object with 404. Connections stay
  instance-wide until checkpoint 3 (owner null, the login flow's owner
  null).
- `ControlService`: `createMaster` (insert-if-empty in a transaction),
  `createUser`, `getUser`, `findUserByUsername` (with the hash),
  `listUsers`, `countUsers`, `setUserPassword`, the four `web_sessions`
  methods; `ensureUser` and the `settings` table are gone.
- `createBackend` takes `mode` (required) and `auth` (test tuning);
  `main.ts` requires `DEMI_INSTANCE_MODE`. The mode does nothing yet
  beyond being named; checkpoints 2 and 3 read it.
- Tests: `__tests__/session.ts` — `openBackend` (shared mode by default;
  sets the master up on a fresh data directory, logs in over a reopened
  one) returning the backend with a signed-in `session` whose `fetch`
  and `socket` carry the cookie (Bun's WebSocket takes headers); every
  test file and the scenario world go through it, and the fixed `local`
  ids became the session user's. `auth.test.ts` has the four cases.
  Backend suite: 92 pass, 2 skip.

### Checkpoint 2: the admin surface (2026-09-03) — delivered

- `http/users.ts` behind `requireAdmin` (a user is 403 `forbidden`):
  `GET` lists every account; `POST { username, password, role }` with
  `role` admin or user, refused 403 unless the actor outranks the role
  (so only the master makes admins), 409 `username_taken`; `PATCH /:id
  { password }` resets a lower-ranked account's password — admin → user,
  master → admin or user; nobody resets the master, nobody resets a
  peer. No deletion. `http/settings.ts`: `GET { mode }`.
- `admin.test.ts`: the rank matrix through real logins after each reset;
  the mode read back in both configurations, 401 without a session.
  Backend suite: 94 pass, 2 skip.

### Checkpoint 3: instance-mode enforcement (2026-09-03) — delivered

- `vault/scope.ts`: `connectionOwner(mode, userId)` (shared → null,
  isolated → the user), `canConfigureProviders(mode, role)`,
  `ownerFitsMode`. One scope runs through everything that touches a
  connection: `vault.list(scope)` / `control.listConnections(scope)`
  (`{ ownerUserId }` or `'all'`), `assembly.catalog(owner)`, the
  connections route (writes 403 for a shared-mode user; a connection
  outside the scope 404 on delete/test), the subscription login flow
  (owner per `start`, `status` visible to its scope), the conversation
  PATCH (a `connectionId` outside the scope is 404), and
  `resolveProvider` (owner mismatch ⇒ no provider for the session).
- `GET /api/usage/instance`: shared mode, admins; the ledger per user
  with the same aggregation (`control.listAllUsage`). 403 in isolated
  mode.
- `createBackend` refuses to start when any stored connection's owner
  contradicts the mode: the mode is fixed once providers are
  configured.
- `mode.test.ts`: shared (user refused, uses the instance connection,
  own ledger, the instance ledger by user, restart under isolated
  refused) and isolated (own listings and catalog, another user's
  connection 404 everywhere, the selection guard, ledgers apart).
  Backend suite: 96 pass, 2 skip.

### Checkpoint 4: the tenant-isolation matrix (2026-09-03) — delivered; M12 closed

- `isolation.test.ts`: alice with a paired runner, a workspace on it, a
  conversation with a grant and an attachment; fifteen routes naming her
  objects tried by bob and by the master (transcript, rename, archive,
  the grant set both ways, the workspace-file drop, bob's own
  conversation switched to her workspace or granted her device, device
  revoke and browse, workspace rename/delete/create-on-her-device): 404
  each; her stream refused to both; their lists empty. The attachment
  reference inbound already checks the owner (`attachment-refs.ts`); the
  blob route stays content-addressed — the hash is the capability, and it
  is only ever handed out inside an owned transcript.
- The matrix found one defect: revoking a device under a workspace
  pointer failed on the foreign key with a 500. Final state: `DELETE
  /api/devices/:id` is 409 `device_in_use` while workspaces point at the
  device (the mirror of the workspace rule), and `deleteDevice` drops
  the device's grants with it. Then the runner is refused for good, and
  re-pairing the machine is a fresh claim — by any user; the old id is
  gone for everyone.
- `backend.md`'s endpoint table is the frozen surface: the `commands`
  row went (the manifest rides the runner socket since M9; no such
  routes exist), the setup and password rows came, the scoped rows say
  so. Backend suite: 97 pass, 2 skip.

### Rename: connection → provider (2026-09-03) — delivered

Ruling: "connection" was demi-next's own word for a provider type
configured with one credential; the core packages already call that
instance a provider and the backend was passing the connection id as the
`providerId`, so the two names meant one thing. The word is gone from
the product: table `providers` (column `provider_type`), `provider_id`
on conversations and the ledger, `/api/providers` (request and response
field `providerType`, body `{ provider }`, list `{ providers }`, code
`provider_not_found`), `ProviderVault` / `ProviderEntry` /
`ProviderRecord` / `ProviderScope` / `CatalogProvider`, `providerOwner`,
the `providers page`. "Connection" survives only where it means a
socket (the runner registry, the transfer broker, the Firecracker API
wait). The frozen table in `backend.md` carries the new names; M13
starts from them.

### Provider entries and the vendor catalog (2026-09-03) — delivered

Topic raised by the user after the rename: what the providers page's list
really is. Ruling: the list's items are provider entries — one runtime
family (`providerType`) with one credential, an endpoint, and a model
source — and the families and the vendors are two different dimensions.
Subscription families (claude-code, codex, grok-build) allow exactly one
entry per scope; API-key families allow any number, each with its own
label. The vendor catalog is models.dev, fetched live by the backend and
never stored: each vendor's `npm` field (the client package opencode
loads for it — the only protocol tag in that data, and what opencode
itself decides by) maps onto our four families
(`@ai-sdk/openai-compatible` → openai / chat-completions, `@ai-sdk/openai`
→ openai / responses, `@ai-sdk/anthropic` → anthropic, `@ai-sdk/google` →
google); vendors on other packages are not offered. A hand-maintained
vendor table was proposed and rejected as the sillier option. Sub-rulings:
(1) vendor baseUrl and model lists stay live, an entry stores only the
vendor id; (2) native openai / anthropic / google key entries take their
model list from models.dev too — the packages' static lists stay for the
local products; (3) entries are editable: `PATCH /api/providers/:id`
(label, baseUrl, apiKey, model list; a subscription entry only its label);
(4) labels are not unique; (5) GitHub Copilot is not offered — it needs
its own auth, nothing in the code ever supported it, so the catalog
excludes it. The models.dev client (fetch, cache, entry mapping) moves
from `provider-claude-code` into `@demicodes/provider`, which the Claude
Code catalog reuses with its version filter. This corrects the M12
frozen table before M13 consumes it: `GET /api/providers/catalog`
(vendors + subscription families with a configured flag), the POST body
in two shapes (from the catalog, or a custom endpoint naming the family),
the PATCH row, 409 `provider_exists` on a second subscription login.
Delivered: `@demicodes/provider`'s `models-dev.ts` (zod schema of the
parts read, the cached fetch, entry mapping; the Claude Code catalog
filters one shared snapshot), the backend's family registry with
credential kinds, `llm/vendors.ts`, the vault `update`, the routes above,
`llm.test.ts` over a models.dev fixture served by the test fixture's
backend. Backend suite: 98 pass, 2 skip; the Claude Code catalog tests
green over the shared client.

### Pipes: one streaming primitive for rpc stdin and stdout (2026-09-04) — rulings

Raised while checking whether a conversation can copy files between its
target and a granted host. It can, by `tar | demi host shell --id`, but
the two directions were carried by unrelated mechanisms and neither was
a stream:

- stdin of every `rpc` command (not only `host shell`) was buffered
  whole three times: the command-mode process read the pipe to its end
  (`command-loader` tree), sent it as one `rpc_call` frame on the runner
  socket, and the backend collected it again as bytes plus a UTF-8
  decoding (`stdinOf` in `@demicodes/shell`); `host shell --id` then
  wrote the whole block to the far job with `job_stdin`. Frame size
  equalled payload size on two runner sockets, and the backend held two
  copies. Data frames of that size on the socket are exactly the
  head-of-line risk `runner.md` § Wire rules records.
- stdout of `host shell --id` was a transfer of a finished file: the far
  job ran to exit, then its stdout file was `PUT` to the caller. Correct
  bytes, no streaming, disk on the far host sized by the payload.
- stdout of an ordinary `rpc` command streamed as `rpc_output` frames on
  the socket — a third carrier for the same concept.

Ruling: a pipe is one primitive, fully isomorphic and fully streaming.
Every `rpc` command's stdin and stdout is a pipe, and so is a job's fd
when `host shell --id` attaches it; a pipe whose ends are in different
processes is one HTTP exchange brokered by the backend (`PUT` from the
source device, `GET` by the sink device, piped in flight, nothing held);
a pipe with both ends in the backend is one `AsyncIterable`. The runner
socket carries only the control frames naming the ends. stderr stays a
view and live stdin stays interactive input — neither is a pipe, both
ride the socket, both are small by nature. Rejected: streaming the pipe
over socket frames (keeps the head-of-line risk the wire rules already
name, and leaves two carriers); fixing memory first and the carrier later
(the first is a subset of the second — rework). `transfer_receive` had no
caller since the upgrade moved to `mke2fs -d` and is dropped rather than
generalised into a file end.

Protocol (next version): `rpc_call` carries `stdin: boolean` and no
bytes; `rpc_pipes { callId, stdin?, stdout }` names the ends before any
output; `job_start` gains optional `stdin` / `stdout` pipes; `rpc_output`
carries the stderr view only; `pipe_done` replaces `transfer_done`;
`rpc_transfer`, `transfer_send`, `transfer_receive` go. The local relay's
`rpc` frame carries `stdin: boolean` followed by `pipe` / `pipe_end`
frames; the runner streams them into the `PUT` with backpressure. Routes:
`/api/pipes/:id`. Contract: `CommandContext.stdin` becomes
`AsyncIterable<Uint8Array>`; `stdinField` leaves are collected by the
loader. Records rewritten: `runner.md` (message tables, § The local
relay, § Pipes replacing § Transfers, § Wire rules), `commands.md`,
`backend.md`, `overview.md`, `scenarios.md`. Implementation follows as
its own checkpoint over `runner-protocol`, `shell`, `command-loader`,
`runner` and `backend`, with the M9 transfer tests re-pointed at pipes
and a push-direction copy added beside the pull.

### Attached hosts (2026-09-04) — rulings

Asked whether a conversation can work on several hosts at once. The
main-host model stands: one execution target where the `bash` tool runs,
which the agent never switches. The M11 grant set was already the second
half — hosts the conversation may reach through `demi host shell` — but
framed as a permission, keyed by device id, always starting in home, and
mentioned to the model only in the switch announcement. Ruling: the grant
set becomes **attached hosts**, a first-class part of the conversation.

- Table `conversation_hosts (conversation_id, device_id, name, cwd,
  attached_at)`, primary key on the device (one attachment per device per
  conversation), `UNIQUE (conversation_id, name)`. It replaces
  `conversation_host_grants`.
- `name` is for the model and the user; the identity stays the device.
  Seeded from the device's hostname, suffixed within the conversation on a
  collision (hostnames are not unique), renamable. Considered and dropped:
  a free per-conversation alias distinct from the hostname (the hostname
  is the name people already use), and relying on the hostname alone (not
  an identifier).
- `cwd` is where work on that host last stood — written back from the
  job's exit after every `shell --host`, the start of the next. Not a
  scope or a permission. Considered and dropped: a user-set directory per
  attachment (only an initial cwd, and it invited attaching a device twice
  with two directories, which made the set ambiguous).
- No copy verb. `tar | demi host shell --host` over a pipe is the idiom;
  a `demi host cp` was proposed and rejected — it would restate `tar`'s
  semantics and, on a hostless main host, need a tar codec in the backend
  where the ordinary upgrade rule already applies.
- `demi host shell --id` becomes `--host <name|id>`; `/api/conversations/
  :id/grants` becomes `…/hosts` with `PATCH …/hosts/:deviceId { name }`;
  a change to the set is announced at the next turn boundary like a
  switch.

Records rewritten: `sessions-and-targets.md` (§ Attached hosts replacing
§ Host grants, § Switching), `commands.md` (§ The `demi host` group),
`backend.md`, `storage.md`, `overview.md`, `product.md`,
`managed-hosts.md`, `runner.md` (the flag in § Pipes). Implementation
follows the pipes checkpoint; the split.test grant cases re-point at
attachments, with name seeding and collision, cwd write-back, rename,
and the announcement covered.

### `tar` as a tinybash builtin (2026-09-04) — ruling

Follows from attached hosts: with `tar | demi host shell --host` as the
copy idiom, a hostless conversation pulling a directory from an attached
host would have hit the upgrade on `tar` and acquired a machine only to
receive files. Ruling: `tar` joins the builtin table, admitted by
structure (the wire format of a cross-host copy) rather than by corpus
frequency, and the earlier note that the upgrade rule covers this case is
withdrawn. Ruling alongside: the attachment set is independent of the
main host's state — hostless plus attached hosts is not a product shape
anyone designs for, but capability is never narrowed to the expected
combinations, and the record now says so. Whitelist `c x t`, `-f`
(default `-`), `-C`, `-v`, `-z` over
the platform's gzip stream; `-j -J` and the rest refused. Three
consequences recorded in `tinybash.md`: archives are compared by listing
and extracted tree, not bytes (name-order entries, `demi` as owner); a
link entry on extraction is the third run-time failure that is not an
upgrade, keeping the tree link-free; extraction paths follow GNU's own
rules inside the store, which is the namespace. `runner.md` § Pipes'
hostless example now shows the pull. Implementation lands with the
attached-hosts checkpoint, in `@demicodes/tinybash` beside the other
builtins, with the corpus cases for `c`, `x`, `t`, `-C`, `-z`,
`--strip-components`, the link refusal and the `..` member.

Closing rulings (2026-09-04): `--strip-components=N` is whitelisted on
`x` — the model's habitual form for unpacking a downloaded source tree.
Explicit attaching is bounded by the product surface, not the backend:
the route accepts any device the user owns, the devices page offers user
hosts. Switching the main host to an attached host removes its row; a
host is main or attached, never both. The model learns of attached hosts
through `demi host list`, whose help is in the system prompt like every
root command's — no extra tool-description text.

### Pipes checkpoint (2026-09-04) — delivered

The pipe primitive as `runner.md` § Pipes records it, bottom up.

- **tinyjs** (ABI 2): `fs.pipe()` — a bounded in-memory pair over
  `tokio::io::duplex` (not `simplex`: closing either end must be seen by
  the other as EOF or `EPIPE`, which the split halves of a simplex never
  signal — the first draft hung on exactly that); `httpRequest` bodies
  from `{ handle }`, the handle consumed by the request and the body sent
  chunked, boxed without `Send` so a handle-table stream can be a body;
  `spawn`'s `tee.stream` adding a third destination to the tee, the full
  stdout as a handle whose backpressure reaches the child, a closed reader
  stopping that copy and nothing else. Conformance cases for each; the
  protocol fixture's `hello` had lacked `homeDir` and was failing before
  this work.
- **Protocol** (v5): `rpc_call.stdin` a boolean; `rpc_pipes`; `job_start`
  with `stdin` / `stdout` pipe refs; `rpc_output` stderr only; `pipe_done`;
  the four transfer messages gone.
- **shell / command-loader**: `CommandRunContext.stdin` is the pipe as an
  `AsyncIterable`, `null` when there is none; only a `stdinField` drains
  it before parsing; `DispatchIO.stdin` optional (absent: fd 0 is not a
  pipe); `RpcInvocation.stdin` the stream or `null`. `CommandStdin`,
  `stdinOf`, `emptyStdin` removed.
- **runner**: the local relay carries the pipe as `pipe` / `pipe_end`
  frames behind an `rpc` frame with `stdin: boolean`; the relay server
  feeds them through a `ByteChannel` (new in `@demicodes/utils`: one
  chunk in flight, `push` resolves on take) into the `PUT` that
  `rpc_pipes` names, streams the `GET` body back as `output` frames in
  the call's write chain, and reports `pipe_done` per end. `PipeClient`
  replaces `TransferClient`: `put(url, stream)` pumps into an `fs.pipe()`
  whose read end is the request body, `get(url)` streams the response.
  The job table attaches a job's fd 0 to a `GET` and its fd 1 to a `PUT`
  of the tee's stream; a refused end is released so the child never
  blocks on a reader that left (`readHandle` closes on `return()` before
  the first read too). Command mode passes no pipe when fd 0 is the
  job's live stdin. `pipes.test.ts` runs 3 MB through both ends of a job
  on the tinyjs runner and both refusals.
- **backend**: `PipeBroker` replaces the transfer broker — an end may be
  left open at minting and fixed later by `stream()` / `writer()` (this
  process, pull-based, `highWaterMark: 0` so the writer waits for the
  sink's pull) or `sinkTo` / `sourceFrom` (a device); `/api/pipes/:id`;
  the relay mints a call's pipes, sends `rpc_pipes` first, runs the
  handler with the pipes on its `CommandIO`, and sends `rpc_exit` after
  the stdout pipe drained; `host shell --id` names the far ends of a
  relayed caller's pipes as the job's device (device to device through
  the broker, zero bytes here) or, hostless, feeds and drains the job's
  pipes from its own streams. A sink stopping early counts as drained,
  the way a closed pipe does. `RemoteHost.startJob` takes the refs.
  Tests: `pipe-broker.test.ts` (late ends, in-process ends with
  backpressure, timeout, device loss, early sink); `host-shell.test.ts`
  now copies both ways with the wire audit on each; the scenario
  invariant counts pipe ends named against `pipe_done` reports, waiting
  for reports that trail the turn. Backend suite 102 pass; the root suite's
  five failures reproduce on the committed tree (repository-scan
  timeouts, the catalog-label check from the provider-entries commit, a
  `.bashrc` on this machine).

### Attached hosts checkpoint (2026-09-04) — delivered

The grant set becomes attached hosts as `sessions-and-targets.md`
§ Attached hosts records.

- **Storage**: `conversation_hosts (conversation_id, device_id, name, cwd,
  attached_at)`, primary key on the device, `UNIQUE (conversation_id,
  name)`, an index on the device; `conversations.hosts_changed` marks a
  change for the next turn's announcement. `cwd` is nullable: `null` is
  "its home", until a shell there ends somewhere. `attachHost` is
  idempotent and seeds the name from the hostname, suffixed `-2`, `-3`, …
  while taken in the conversation; `renameAttachedHost` answers
  `renamed` / `name_taken` / `not_attached`; `setAttachedHostCwd` is the
  write-back. The switch write attaches the departed device at the
  directory it was left at (a workspace's path; a managed host's home)
  and detaches the arriving device, in the same compare-and-set — a host
  is main or attached, never both. Revoking a device drops its rows.
- **HTTP**: `/api/conversations/:id/hosts` — GET (`{ hosts: [{ deviceId,
  name, cwd, online, attachedAt }] }`), POST `{ deviceId }` (any device
  the user owns; 409 `host_is_main` for the conversation's own main
  host), PATCH `…/:deviceId { name }` (409 `name_taken`, 404
  `host_not_attached`), DELETE. `/grants` is gone.
- **`demi host`**: `list` prints name, id, online, the directory shells
  start in, `(main)` / `(attached)`; `shell --host <name|id>` replaces
  `--id`, resolving the name first and the device id second, starts an
  attached host's shell where the last one ended, and writes the job's
  exit directory back.
- **Announcements**: the switch block names the departed host as it is
  attached (`stays attached as "<name>"`, `demi host shell --host <name>`)
  and ends with the attached-host line; a change to the set without a
  switch is announced as `[Attached hosts changed]` with the same line;
  both clear the pending state together.
- **Tests**: `switch.test.ts` (attachment with the departed directory,
  the row removed when the target moves onto the attached device, the
  hosts API including a managed host, rename uniqueness, the change
  announcement, name seeding and collision at the control level),
  `host-shell.test.ts` (names in `--host` and `list`, the directory
  carrying between shells on an attached host, the id accepted too),
  `isolation.test.ts` and the s6 / s10 scenarios re-pointed. Backend
  suite 99 pass.

### `tar` builtin checkpoint (2026-09-04) — delivered

`tar` in `@demicodes/tinybash` as `tinybash.md` § Builtins records it:
`builtins/tar-format.ts` (ustar/GNU/pax headers, GNU long names, the
block reader, the record padding) and `builtins/tar.ts` (both option
spellings, `c` / `x` / `t`, `-f`, `-C`, `-v`, `-z` over
`CompressionStream`, `--strip-components` on `x`, GNU's messages).
`TinybashFs` gains `chmod`, which extraction needs beside `utimes`.

- Members are written in name order with the session identity as owner
  and the archive padded to GNU's 10 KiB record; extraction applies the
  umask (022) to modes as GNU does without `-p`, restores mtimes, and
  defers directory modes and times to the end.
- GNU's wording was taken from GNU tar 1.35 itself (the lima `fc` VM,
  the corpus's bash): `Removing leading \`/' from member names` keeps
  the literal backtick of tar's source whatever the locale; a member
  with `..` gets `Removing leading \`a/../' from member names` once per
  prefix and `Member name contains '..'`, and is skipped; an unreadable
  `-f` is `Cannot open: … / Error is not recoverable: exiting now`; a
  stream shorter than a block is `This does not look like a tar archive`,
  a later cut is `Unexpected EOF in archive`. Every member failing still
  writes the archive's end, as GNU does.
- A link entry on extraction is refused with `Cannot create symlink to
  '…': Operation not permitted` (a hard link: `Cannot hard link to`),
  the third run-time failure that is not an upgrade; `tar notes.txt` (a
  first word that is not options) is outside, as any letter beyond the
  whitelist is.
- Tests: `tar.test.ts` (round trip with modes and mtimes, name order,
  the record size, both spellings, `-z`, `--strip-components`, member
  selection, the error lines, the `..` and link refusals, the system
  tar reading tinybash's archives and tinybash reading its, outside
  cases); nine corpus cases with goldens from GNU tar 1.35 (`tar-*`,
  listings piped through `sort` where GNU's order is the filesystem's;
  `ls -l` on files only, since directory totals are Linux-only); the
  host-shell end-to-end test pulls a directory into a hostless
  conversation through `tar` without acquiring a machine. tinybash suite
  411 pass.

### The Firecracker smoke on the pipes runner (2026-09-04) — delivered

The leftover from the three checkpoints: the guest runner is the packed
tinyjs binary, so the ABI-2 primitives and the new relay only reach a
managed host once the image carries them. Rebuilt and re-run in the Lima
`fc` instance (Ubuntu 24.04 aarch64, 4 vCPU, nested KVM; Firecracker
v1.16.1; kernel 6.1.155 at `/opt/fc/vmlinux`).

- `runner/build.sh aarch64` on macOS (`cargo zigbuild`, musl,
  `guest-roots`) produced the new `demi-runner`; it replaced
  `/demi-runner` in the existing `rootfs.ext4` rather than rebuilding the
  image from debootstrap — the runner is the only input that changed
  since the image was built.
- `real-firecracker.e2e.test.ts` passes in **both** launch modes with
  that runner. It calls no model (the scripted stub answers), so the
  `real-*.e2e.test.ts` gate rule — which exists for real model calls —
  is not what holds it back; `/dev/kvm` is, and the install script's
  `kvm` group only takes effect in a new session (`sg kvm -c`).

| | direct | jailer |
|---|---|---|
| cold provision + first job | 6.5 s | 8.2 s |
| wake + job | 8.5 s | 7.0 s |
| home stored after hibernate | 8.5 MB | same |
| home filesystem after growth | 109 MB | same |

What this validates of the pipes work: the smoke's first guest turn ends
in `demi host current`, an `rpc` command relayed from inside the VM, so
the whole new path ran over the real tap network — the command-mode
process declaring `stdin: false` on the UDS, `rpc_call`, the backend
minting the stdout pipe and sending `rpc_pipes`, the guest runner
`GET`ting `/api/pipes/<id>` from the backend at 172.16.0.1:3277 and
streaming the body back as relay frames, and `rpc_exit` after the drain.
Not covered here (covered by `host-shell.test.ts` on two local tinyjs
runners instead): job-to-job pipes between two devices, and `tar`. The
dev instance's tap pool was left in `jailer` mode by the last install
run; either mode is one `install-managed-hosts.sh --mode …` away.

## Review corrections (2026-09-05) — complete

Scope: the 19 findings against `f6b98a2e`, on
`codex/demi-next-review-fixes`. Web and concrete Sandbox implementations
remain outside this checkpoint. All verification uses scoped tests with
real-model gates unset.

| Finding | Milestone | Final-state correction | Status |
|---|---|---|---|
| 1 | M3/M12 | Blob namespaces belong to users; conversation and HTTP stores resolve the same owner. | verified |
| 2–3 | M5/M6 | Resolve current provider configuration before inference; rebuild process runtimes when their Host changes. | verified |
| 4 | M9 | Order each stdin stream without blocking control-message dispatch. | verified |
| 5 | Pipes | Distinguish pipe endpoint assignment from arrival; stop the arrival timer when connected. | verified |
| 6, 10 | Attached hosts | Carry live stdin and cancellation through RPC to the remote job. | verified |
| 7 | M8/M11 | Serialize concurrent appends to a hostless file. | verified |
| 8 | M4/M9 | Bind each device once after asynchronous handshake preparation and refuse closed connections. | verified |
| 9 | Pipes | Stream stderr concurrently with stdout; only exit waits for both. | verified |
| 11 | M2/M6 | Validate inbound frames before rewriting; isolate each delivery failure. | verified |
| 12 | M8 | Cancel pending stdin reads and report an aborted command as aborted. | verified |
| 13 | M8 | Treat redirection IO failure as the current command's failure. | verified |
| 14 | M8/M11 | Preserve explicitly changed initial environment variables during handover. | verified |
| 15 | M8 | Finish `head -c` immediately when its byte count is satisfied. | verified |
| 16–17 | Tar | Accumulate relative `-C`; fail when requested members are absent. | verified |
| 18 | Provider catalog | Apply the vendor's reasoning replay policy to compatible requests. | verified |
| 19 | M5 | Enforce subscription family uniqueness atomically in storage. | verified |

Tests and conclusions are recorded here as each checkpoint is completed.

### Shared queue primitive — verified

`@demicodes/utils` owns `SerialQueue`: operations on one resource remain
ordered, unrelated queues run independently, and rejection does not poison
the next operation. The generic primitive is shared by file appends,
scoped frame transport and runner stdin dispatch. Its `idle` state allows
owners to release per-resource queues once work finishes.

Verification: `utils/src/__tests__/serial-queue.test.ts`, **1 pass, 0 fail**,
covering ordering, concurrent independent resources, rejection recovery
and idle/settled state.

### Shell semantics checkpoint — verified

Findings 12–17: cancellation closes pending stdin reads and removes signal
listeners; an aborted dispatcher cannot turn the command into a success.
Redirection flush distinguishes filesystem errno failures from implementation
exceptions and flushes other sinks before reporting the command failure.
Environment handover compares values with the initial environment. `head -c`
finishes at the byte boundary. Tar accumulates `-C` and reports unmatched
members while retaining metadata for members already extracted.

Verification: host-virtual lifecycle and tinybash builtin/corpus/session/tar
tests, **424 pass, 4 Linux-only skips, 0 fail**. New test coverage is listed
in `tinybash.md`. The first test run exposed a fixture mistake: spreading a
Host filesystem drops prototype methods; fault injection now uses a Proxy.

### Storage, protocol and provider checkpoint — verified

Findings 1, 7 and 11: `UserBlobStores` binds every media/file consumer to
`blobs/<userId>/<sha256>`; authenticated downloads use the same user store
and `Vary: Cookie`. Root and child session media, uploads, cold/live
transcripts and hostless files share that ownership contract. File appends
serialize their whole read/modify/write operation per path. The scoped
transport validates before metadata writes, resolves references on send
and steer, and recovers its inbound/outbound queues after individual
failures. The final schema and directory shape apply directly; no runtime
historical-data lookup or migration path is added.

Findings 2, 3, 18 and 19: inference rechecks provider scope and the current
vault entry before charging rate limits. Config/label changes and process
Host switches invalidate session runtime reuse. Runtime rebuilds use the
current request's model id, thinking and service tier. DeepSeek gets the
reasoning replay option at assembly. A partial unique index enforces one
subscription entry per owner scope/family, and the login flow puts its
credentials in the final directory before exposing the provider row.

Independent review caught two additional edges within these fixes:
cookie cache variants must separate users, and a delayed pre-edit vault
lookup must not permanently repopulate the provider cache with an old
snapshot. Both have regression coverage. Re-resolving only the spawn
function was insufficient for process providers that retain a CLI between
turns; their runtime identity now includes the stable Host object.

Verification: the 27-file backend batch, **100 pass, 0 fail, 769
assertions**, includes auth/admin/mode/isolation, storage, attachments,
providers/vault, scoped transport, hostless shell, host switching,
runner/registry/pipes/host shell, scenarios S1–S9 and restart R1–R4. The
root storage/frame regression subset is **16 pass, 0 fail**; the final
provider session/catalog subset is **11 pass, 0 fail**. These subsets are
included in the backend batch, not additional unique test counts.

### Runner control and pipe checkpoint — verified

Findings 4–6, 8–10: runner stdin writes/EOF use per-process queues while
control dispatch remains available. Pipe endpoint assignment and arrival
are separate; both arrivals stop the timeout and duplicate HTTP endpoints
are refused. Stdout streaming and stderr delivery share only the socket
writer; the exit waits for stdout drain. Registry hello/claim completion
checks socket/registry liveness and binds the device without an intervening
await; a closed claim cannot leave an unusable device row.

Protocol v6 carries `rpc_cancel` into a handler AbortSignal. `demi host
shell` forwards live input, sends TERM on cancellation and escalates to
KILL after five seconds. The tinybash dispatcher represents live stdin
with `stdinStream` and no finite `stdin`. Local relay calls have a separate
watch connection and job ownership, so cancellation does not depend on
reading a blocked data socket. The RPC is registered before the watch-ready
acknowledgement allows the client to send stdin.

Pitfall: an explicit cancel frame on a second short connection covered
cooperative abort but could not observe SIGKILL of the command process.
The lifetime watch closes on process death; the regression kills only that
process during a 20 MB blocked upload while its parent shell stays alive,
and confirms the far job terminates. Independent review verified the final
handshake ordering and cleanup paths.

Verification: the 19-file runner/protocol/host-remote/backend control/
tinybash/SerialQueue batch, **456 pass, 4 Linux-only skips, 0 fail, 1,797
assertions**. After the final handshake-order adjustment, the affected
backend runner/host-shell tests passed again: **8 pass, 0 fail, 79
assertions**. These batches overlap the shell and backend counts above.

### Regression modules and their intended coverage

| Module | Coverage |
|---|---|
| `utils/src/__tests__/serial-queue.test.ts` | Ordered work, unrelated resource progress, failure recovery and idle cleanup. |
| `backend/src/__tests__/isolation.test.ts` | Cross-user attachment and hostless-file hashes are inaccessible; same bytes uploaded by each user remain accessible only within their stores; Cookie cache variants. |
| `backend/src/__tests__/storage.test.ts` | Concurrent existing/new-file appends retain every byte; failed writes release the queue; session/blob persistence. |
| `backend/src/__tests__/scoped-transport.test.ts` | Malformed frames cause no metadata writes, a failed delivery does not block the next frame, send/steer resolve attachment references, outbound blob failures preserve later delivery. |
| `backend/src/__tests__/session-providers.test.ts` | Delayed stale-cache lookup, current-model runtime rebuild after edits, clone isolation and steering. |
| `backend/src/__tests__/llm.test.ts` | Credential edits/deletion, process Host switching and spawn targeting, DeepSeek thinking/tool replay, provider HTTP behavior, concurrent subscription completion and ready credential pools. |
| `backend/src/__tests__/vault.test.ts` | Atomic per-scope subscription uniqueness, including shared scope; independent API-key entries. |
| `backend/src/__tests__/runner-registry.test.ts` | Concurrent hello, repeated hello, closure during manifest preparation and closure during claim; RPC cancellation/disconnect abort handlers and release live stdin. |
| `backend/src/__tests__/pipe-broker.test.ts` | Quiet/slow active streams survive the arrival deadline, abandoned endpoints expire, duplicate attachment fails, cancellation interrupts active flow. |
| `backend/src/__tests__/host-shell.test.ts` | Two-runner and hostless live stdin, stderr before stdout EOF, far-job abort, blocked uploads and abrupt command-process death. |
| `runner/src/__tests__/control.test.ts` | Blocked job/raw-spawn stdin does not block ping, filesystem work or kill; per-process input order is retained. |
| `runner-protocol/src/__tests__/protocol.test.ts`, `runner/src/__tests__/tinyjs-codec.test.ts` | v6 cancellation message validation and MessagePack round trips on Bun/tinyjs. |
| `host-virtual/src/__tests__/hostless-environment.test.ts`, tinybash builtin/session/tar suites | Findings 12–17 and the live-vs-finite stdin contract; detailed command coverage is in `tinybash.md`. |
| `core/src/__tests__/platform-entrypoints.test.ts` | Package dependency boundaries, production-only helper scans, the allowed models.dev client and prohibited catalog source labels in shared metadata. |

### Final architecture checks — verified

The broader agent/core/utils/shell/command-loader/host-virtual/host-remote
batch first exposed two pre-existing test-rule mismatches: the catalog
guard rejected the models.dev client that the package contract explicitly
assigns to provider, and global source scans included a local reference
checkout outside the production package registry. The guard now enumerates
the existing production package directory table and distinguishes client
implementation facts from shared catalog metadata. Five focused rule cases
cover the allowed client, forbidden source tags and the separate legitimate
quota-cache semantics. No local reference files are deleted or changed.

Final seven-package batch: **380 pass, 0 fail, 2,355 assertions across 40
files**. The architecture guard alone passes all 25 cases. Final
`bun run typecheck` and `git diff --check` pass. All 19 review findings are
implemented and verified; the separate test-rule corrections are complete.
Test totals above are per batch and overlap. Web, concrete Sandbox
implementation, managed/init test suites and real-model calls remain
outside the review and verification scope.

## Main synchronization (2026-09-05) — complete

Target: `feat/demi-next`. The completed review fixes through `d8bb3776`
were fast-forwarded into Next and pushed before integrating `origin/main`
at `92e852ea`. Work continues directly on Next, as requested.

The main-line changes span shell running hints, context-window preview
budgets, the subagent tree/message model, unnamed inherited profiles,
profile validation before archive revival, root-only notification policy,
and releases through 0.24.0. Their runtime behavior is integrated into
Next's product composition and module boundaries.

Conflict decisions: retain Next's removed-package/source decisions and
port behavior into its current modules. `host-remote` is a distinct new
package, so Git's apparent rename from `host-local/package.json` does not
transfer the local package's identity. The lockfile combines main's
dependency updates with Next's package graph and current workspace
versions; `bun install --frozen-lockfile --ignore-scripts` passes.

Server integration passes `maxLiveSubagents` and the shell preview callback
through the facade, transport binding and session assembly; each root gets
an AgentDirectory shared with its descendants. Existing per-root BlobStore
ownership, action-aware Host routing and injected ShellEnvironment remain
the composition contracts.

`AgentDirectory` lives in its own module and follows the root session
across transport takeovers. Unnamed descendants inherit a named parent's
effective prompt/model/commands; restricted children retain that setting
through archive and resume. Recursive checkpoint paths and descendant
restore ordering preserve the full subtree before its parent can settle.

The command loader carries `runningHint` from the real executable leaf
through the manifest into each Host's status view. Runner protocol v7 adds
the per-invocation hint lifecycle; a dedicated local lifetime connection
clears it on completion, abort, disconnect or command-process death,
including while its parent shell remains alive. The existing RPC watch,
cancellation, stdin backpressure and concurrent stderr contracts remain.

Validation:

| Scoped batch | Result and intended coverage |
|---|---|
| Agent/core/utils/shell/coding-agent/command-loader/host-virtual/host-remote | 439 pass, 0 fail, 2,693 assertions across 46 files; includes architecture, nested subagents, model-aware previews, Host ownership and command execution. |
| Backend 27-file regression | 100 cases exercised; 98 passed initially. Two S2 expectations still asserted the generic polling hint for a subagent spawn. They now require the custom no-polling guidance on both targets; the entire S2 module rerun passed all 8 cases with 46 assertions. No implementation regression remained. |
| Running-hint and runner/protocol scoped checks | 95 pass, 0 fail, 482 assertions across 22 files, including actual tinyjs execution, pipe/control regressions, help/validation exclusions, cancellation during hint registration and command-only SIGKILL cleanup. |
| Final static/package checks | `bun run typecheck`, staged/working `git diff --check`, frozen lockfile installation, and all 21 workspace name/version matches pass. |

The subagent module includes 31 cases covering tree messaging, root-only
notification policy, per-parent caps, reserved profile names, revival
validation, recursive persistence and inherited restrictions. The preview
regressions exercise the real AgentServer assembly through a model switch
and live transport takeover. Batch counts overlap. Concrete Sandbox/init
implementations, Web runtime review and real-model tests stayed outside
this synchronization's verification. The fetched main head was rechecked
as `92e852ea` before completing the merge.

## Session architecture proposal (2026-09-05) — design review pending

Scope: the user selected checkpoint consistency, Host/state separation and
execution recovery for design work. Global agent budgets, hostless upgrade
tradeoffs and multi-worker topology are outside this proposal. The user
requires root agents and descendants to be architecturally isomorphic,
with shared mechanisms, explicit ownership and strong maintainability.

Verified current seams before proposing changes:

- `server/open-session.ts` and `subagent/supervisor.ts` each assemble
  sessions, tools, environments and event subscriptions. `LiveSession`
  and `ChildJob` own parallel sets of runtime resources.
- Backend root checkpoints commit blocks and state in one SQLite
  transaction. Child checkpoints use `hostAgentSessionStore`, whose
  per-key writes do not make a checkpoint atomic. Child close metadata
  is written separately from its final checkpoint.
- Session opening resolves a provisional Host before loading state;
  child Host resolution substitutes the root identity. Host-bound stores
  carry conversation state even when execution occurs on another target.
- `commitTranscript` schedules a throttled write; awaiting it is not a
  durable pre-dispatch barrier. Restoring pending tool calls substitutes
  interrupted errors without consulting an execution receipt.
- Remote disconnect finishes local job handles with a synthetic failure;
  the current shell status contract has no explicit unknown outcome.

The standalone proposal is `docs/demi-next/session-runtime.md`. It defines
one node assembly and lifecycle, explicit scope/session/parent identities,
flat per-session rows in the conversation database, atomic domain commits,
Host-independent storage, uniform node events, transactional message and
completion delivery, and operation evidence for recovery. It distinguishes
a completed tool observation from a still-running command, cancellation
intent from confirmed termination, and atomic domain deduplication from
arbitrary external effects. It does not promise exactly-once shell execution.

Status: proposal written for user review; no runtime implementation or
canonical package-boundary change. No tests were run for this documentation
checkpoint. Runnable call sites and storage/protocol interfaces were read
directly; documentation diff/whitespace checks passed. Implementation
acceptance coverage is specified in the proposal, not reported as passing.

## Review round 2 (2026-09-05)

A full read of the M0–M12 implementation against the records, after the
19-finding round above. Two kinds of finding: four groups with one root
cause each, fixed by one mechanism per group, and a set of independent
point fixes. Each group is a checkpoint; status below.

| Group | Root cause | Findings it covers | Status |
|---|---|---|---|
| 1 | The managed-host lifecycle kept its state in memory only: no record of running VMs on disk, no reconciliation against processes and images, several exit paths (destroy, backend close, crash) that did not save the home. | VMs orphaned by a backend restart and a second guest booted over their image; destroy dropping the work since the last checkpoint; a failed shrink leaving an image the next wake booted anyway; a guest dying during boot failing only at the boot timeout. | delivered |
| 2 | The hostless → machine upgrade modelled as a per-(session, Host) shell decoration in the composition root rather than a conversation-level target transition. | Named shells and running commands unusable after the upgrade; a subagent's outside script failing the upgrade; a failed upgrade's retry booting the stale first image and dropping the hostless work in between; the managed → workspace → hostless loop. | delivered |
| 3 | No single declaration of a job's environment per target kind. | `PATH=/usr/bin:/bin` forced on user hosts; `$USER` empty on managed hosts. | delivered |
| 4 | Guards and documents written twice. | The boundary test admitting `shell → tinybash`; an unused backend dependency; scripts and path mappings pointing at deleted files; stale entry names in `backend.md`. | delivered |

Point fixes (independent): the blob route's reflected content type; device
revoke on a managed row; the hostless quota walk; conversation database
handles; the provider Test on a process provider; the `open` frame's
provider scope; web session and login-limiter growth; the offline
placeholder identity. Status: pending.

### Group 1: the guest lifecycle — delivered

`FirecrackerProvisioner` keeps one invariant (`managed-hosts.md` § Home
persistence): a working image exists only while its guest runs or until
its save succeeded, and the store holds the current home whenever none
exists. Every transition of one guest runs through a per-guest
`SerialQueue`; hibernate, destroy and `close` all end in the same
`stop → save`; `save` failing keeps the image and refuses the next boot
until it succeeds. `startVm` writes `<runDir>/<vmId>/vm.json` (pid, owner)
as soon as the process exists; `reconcile()` — the seam's new method,
awaited by `createBackend` before it serves — kills every recorded process
still alive (through the helper in jailer mode), removes its directory,
saves every working image left behind and deletes stale checkpoint copies.
`ManagedHosts.destroy` hibernates first; `ManagedHosts.close` hibernates
every running guest and then closes the provisioner; the archive route
destroys before it sets the flag. A death during boot rejects the in-flight
boot (`guest_died`) instead of waiting for the timeout.

Verification: `firecracker.test.ts` gains the reconcile and destroy cases
over injected image tools and process control (a still-running VM killed
and its image saved, a dead one's directory removed, a failed save kept and
refusing the next wake, destroy saving before it forgets); S10–S12 pass
unchanged in behaviour (the fake's `reconcile` is a no-op).

### Group 3: the environment table — delivered

One table, declared once: `HOSTLESS_ENV` in
`conversation/hostless-shell.ts` (`PATH`, `SHELL`, `LANG`; `HOME` and
`USER` are the shell's and the guest's own) is what the hostless shell
starts with and what the backend names in every `job_start` to a managed
host, so `echo $PATH` and `$LANG` agree across the upgrade. A user host's
shell starts with nothing of it: the backend names only the `DEMI_*` ids
(`shellOptionsFor` in the composition root picks the table by the device
kind behind the Host; the `AgentServer` no longer carries a `PATH`). On the
runner a job's environment is the device's underneath and the backend's on
top — `{ ...deviceEnv, ...job.env }` in `JobTable` — where `deviceEnv` is
the environment the runner was started with on a user host and the guest's
login table (now with `SHELL` and `LANG`) as PID 1; the narrow `PATH`/`HOME`
fallback stays for raw spawns, whose spawner owns the environment it named.
`runner.md` § Jobs and `sessions-and-targets.md` § What moves carry the
rule and the table.

Verification: `runner/src/__tests__/jobs.test.ts` (a device entry reaches
the job, a backend entry wins), S7 (a user host's `job_start` names no
`PATH` or `HOME`), S11 (a managed host's jobs carry the table and the
model sees the same `$PATH` after the upgrade); the host-remote,
host-virtual and backend suites pass.

### Group 2: the conversation target — delivered

`conversation/target.ts` (`ConversationTargets`) owns what `backend.ts`
and `target-switch.ts` held between them: the resolution of the three
states to a Host (with the owner check and the wake of a managed device),
the switch, the upgrade, and the rule that a conversation with a machine
of its own — `devices.owner_conversation_id`, wherever its main host is —
has no hostless entrance. The composition root wires and mounts again.
The upgrade is one promise per conversation, joined by every session that
asks; the conversation behind a Host is recorded when the Host is handed
out, so a subagent's outside script (its `agentSessionId` is its own)
upgrades its root's conversation. `ManagedHosts.provisionFresh` is the
first boot that leaves nothing behind — device row, guest and image gone
on failure — shared by the upgrade and the Cloud workspace; `provision`
refuses an owner that has a machine. The shell side: `UpgradingShell` is
the one environment a session keeps across the upgrade — the factory
returns it for the machine's Host too and hands it the machine's
`RemoteShellEnvironment` (`attach`), whether its own upgrade or another
session's made the machine — and adopts each hostless shell there the
first time it is used (`RemoteShellEnvironment.adoptShell`, from
`HostlessEnvironment.handoverOf`, which now names the shell): same id, the
machine's counterpart of its directory, its variables, the session's
default staying the default. No script prefix any more, so line numbers
in bash's messages are the script's. `LiveSession` compares environments
by identity, since one object may now stand behind two Hosts.

Verification: S11 gains four cases — a named shell continues on the
machine under its id, in its directory and with its variables, and the
default stays the default; a subagent's outside script upgrades the root's
conversation and both continue there; a first boot that fails leaves no
device row and the retry moves the tree as it stands then (a flaky
provisioner); the return to hostless is refused from the machine and from
a workspace it moved to. The M6 acceptance's attachable managed device is
now owned by another conversation, as the case it tests requires.

### Group 4: guards read their documents — delivered

`platform-entrypoints.test.ts` no longer carries copies of the package
set, the entry list and the dependency graph. It reads the root
manifest's `workspaces` for the packages, each package's `exports`
(`development` condition) for the entries a specifier resolves to, and
the ```text block under `docs/package-boundaries.md` § Production
Dependency Graph for the edges. Three checks fall out of that: the
documented graph and the manifests' `@demicodes/*` dependencies are the
same graph (every package has a line, each line equals its manifest), the
root `tsconfig.json` `paths` are exactly the subpath entries (roots go
through the wildcard, so a root entry is `src/index.ts`), and the root
scripts name paths that exist with the `test` script covering every
package that has tests. What the derived guards found on their first run:
the doc's `provider -> core` line omitted utils; the backend declared
`@demicodes/tinybash` without importing it; the `@demicodes/shell/host-fs`
alias (tsconfig, test, shell README) pointed at a file that no longer
exists, while `agent/client`, `provider/testing`,
`provider/credentials-pool` and `tinybash/testing` had no alias; the
`web:server` script ran a file that does not exist; the backend was
absent from the root `test` script. `docs/demi-next/backend.md` § Packages
named shell entries (`hostless`, `node`) that live in host-virtual.

## Session proposal readability (2026-09-05) — revised for review

The user found the session architecture proposal too long to evaluate.
`session-runtime.md` now presents three decisions: uniform agent nodes,
session-owned atomic storage independent of Host, and evidence-based
execution recovery. Each states its rationale and a falsifiable acceptance
example; storage and recovery also state their costs. Detailed API, schema
and module prescriptions are left for implementation design after these
boundaries are agreed. This is a documentation-only revision; no runtime
tests were run. Whitespace checks passed.

### Point fixes — delivered

The single-site findings of the round, each with its test:

- `GET /api/blobs/:sha256` honours `?type=` only from a fixed list of
  render-safe image, video, audio and PDF types; anything else is an
  `application/octet-stream` attachment, and every response is `nosniff`.
  A blob's bytes are the uploader's under a predictable hash, so
  `?type=text/html` was a stored XSS in the backend's origin.
- `DELETE /api/devices/:id` answers 404 for a managed host: its row is the
  lifecycle's and goes with destroy.
- The hostless quota is the backend's to measure: `VirtualFsBackend.usage()`
  is one `SUM(size)` over the files rows (a directory walk in the test
  backend), instead of `VirtualHost` statting the whole tree on every write.
- `ConversationStores` keeps an LRU of at most 64 open SQLite handles behind
  stable per-conversation database objects that reopen on demand; a cold
  history read no longer pins a file descriptor for the process's lifetime.
- The provider Test button refuses a provider whose transport runs on the
  execution target (`requiresProcessCapableHost`): it would have started the
  CLI on the backend machine with the vault's token.
- The stream's `open` and `set_provider` frames record a provider only when
  the user may name it — the PATCH route's rule, now `visibleProvider` in
  `vault/scope.ts`, shared by both.
- The workspace-file drop writes into the Host's own working directory,
  whichever target: the workspace path, the hostless home, the machine's
  home — not a constant that happened to equal the guest's home.
- Every login sweeps expired `web_sessions` rows; the login limiter forgets
  a name `lockMs` after its last failure, so a spray of usernames is bounded.
- `RemoteHost.attach` takes the runner's identity, so a Host made while its
  runner was offline sheds the placeholder at the first hello.
- The stray `demi` symlink at the repo root (to another user's home) is gone.
- `scripts/build-llms.ts` named five documents that no longer exist, so the
  `llms` CI step failed; the list and the links to the deleted guides
  (`SECURITY.md`, the agent and shell READMEs) now name only what exists.

## Session tree (2026-09-05) — delivered

The session architecture proposal (`session-runtime.md`, two revisions
above) was reviewed against the code and accepted on all three decisions;
the proposal file is gone and its content lives as final-state design in
`docs/subagent.md` (§ Persistence, § Runtime), `sessions-and-targets.md`
(§ The session, § Recovery after a backend restart, § What persists where),
`storage.md` (the conversation schema, § Pluggability), `backend.md` and
`package-boundaries.md`. What the review changed or bounded:

- **Uniform nodes.** Verified: root and child were assembled in two places
  (`server/open-session.ts`, `ChildSupervisor.assembleJob`) with two
  persistence paths and two restore paths; `subagent.md` claimed AgentServer
  was the only place instantiating `AgentSession` while the supervisor did
  too. Final state: one assembly (`node/`), the supervisor a relationship
  module that asks it for children; the differences are configuration and a
  two-flag lifecycle policy (resume interrupted, close when done).
- **Session-owned storage.** Verified: the backend's root checkpoint was one
  SQLite transaction while children went through `hostAgentSessionStore`'s
  per-key writes; the root needed a provisional Host to reach its store; a
  child's close row was written after its final checkpoint with the parent's
  wakeup living only in memory. Final state: the `AgentTreeStore` contract
  (node rows, per-node journals, create/save/close as commits, completion
  delivery recorded by the parent's own save), the backend's realization
  over `nodes` and `blocks(node_id, idx)`, no Host in the storage path. The
  proposal's "close commits the final state and the completion together" is
  realized as two commits — the session's final flush, then the close row —
  because the restore rule closes a quiescent live node with the same result,
  which makes the two orders indistinguishable; cheaper than teaching the
  session to hand its pending journal to a foreign transaction.
- **Recovery by evidence.** Verified: `recovery.ts` already treats an
  executing tool call as landed and `fromCheckpoint` completes it with an
  interrupted error, so the framework never replayed; what was missing was a
  durable dispatch — `commitTranscript` throttles, so the executing block
  could be lost with the process. Final state: the checkpoint is flushed
  before a tool runs. Bounded: "confirm it is still running and keep
  observing" is impossible while the runner kills jobs on disconnect and the
  guest reconcile kills leftover VMs; both are deliberate designs, and
  changing them (jobs surviving a socket drop, re-adopting a running guest)
  is a separate decision, logged under open items.

Also folded: the root's queued messages are restored (they were dropped);
children get the harness's `resolveReferences` and `lifecycle` hooks the
root had; the `list_conversations` frame goes with the Host-store listing
(the backend's conversation index is the list; `web-ui`'s use of it was dead
code from the deleted local product).

### Session tree checkpoint — delivered

The code followed the documents in one checkpoint:

- `@demicodes/agent`: `node/assemble.ts` is the one place a node comes to
  exist (loaded from the tree store or created with its first checkpoint;
  supervisor, command tree, tools, harness runtime, session); `node/node.ts`
  is the runtime aggregate with the per-Host environments and the lifecycle
  policy (`ROOT_POLICY`, `CHILD_POLICY`); `ChildSupervisor` is the
  relationship module and asks the assembly for children through an injected
  `assemble`; `LiveSession` is the root node's transport view. `types.ts`
  carries `AgentNodeRecord`, `AgentNodeClose` and `AgentTreeStore`;
  `store/tree-store.ts` the completion-message id and which completions a
  checkpoint carries; `testing.ts` (`@demicodes/agent/testing`) the
  in-memory store. `hostAgentSessionStore`, `open-session.ts`, the
  `list_conversations` frame and `AgentClient.listConversations` are gone;
  `AgentServer` takes a required `store` factory and no blob store.
- The session: the turn loop flushes the checkpoint before a tool batch
  runs (`persistNow`), and the boundary flush now happens after the phase
  turns idle, so a checkpoint saying otherwise is one the process died in
  — the root's continuation and a child's quiescence both read that.
- `@demicodes/backend`: `storage/tree-store.ts` realizes the contract over
  `nodes` and `blocks(node_id, idx)` (conversation migration 001 rewritten;
  a pre-release data directory is recreated), `ConversationStores.treeStore`
  hands it out, the cold transcript read is the root node's rows.
- `web-ui`: the dead server conversation list is gone with the frame.

Pitfalls met: (1) with the flush moved after `setPhase('idle')`, the
error frame of a failed action briefly followed the idle phase, and clients
that settle an action on the phase resolved instead of rejecting — the
error is announced before the phase turns idle again; (2) the pipeline test
asserted one write per action, now one per tool dispatch plus the boundary;
(3) S3's runner case asserted the child's result in the very last status
view, while the status contract delivers stream deltas per view — the extra
flush shifted the poll cadence enough for a poll to land between the
runner's output and exit frames; the assertion now reads the deltas as a
whole; (4) the first cut of completion delivery marked a completion
delivered whenever the parent was busy, which at restore lost a child that
closed while the parent was already handling another wakeup — the rule is
now: the spawn command's exit carries it only while that command awaits the
child in this process (`attended`), otherwise the parent gets the message,
queued if busy.

Verification: agent 264 (tree-store contract, the dispatch barrier, the
three restart cases in `subagent.test.ts`), backend 130 (the SQLite
realization against the same contract, all scenarios), runner 19, core 30
(the guard now names `node/assemble.ts`), host-virtual 32, coding-agent 32;
`typecheck` and `typecheck:web` clean.

## M13 groundwork — web-gallery integration (2026-09-05)

Status: integrated the squashed web-gallery branch into demi-next. The gallery and
shared session UI are present; M13's backend-connected web product remains planned.
The virtual MenuItem forwards caller attributes to its row container, preserving
absolute positioning and translation when the menu scrolls.

Integration pitfalls: retain demi-next's package graph and scripts, keep the removed
REPL and embedded web server absent, and regenerate the lockfile from the merged
workspace manifests. Register the gallery alongside the other browser packages in
the manifest-level boundary checks; its sample code strings are not production
imports. The activity kind belongs to the turn-flow TypeScript contract and is
consumed by the Vue slot component. The user requested deleting the standalone
web-ui test coverage document. Main was restored to 92e852ea after the gallery was
mistakenly merged there; gallery remains one squashed feature commit.

Verification: root and all three browser typechecks pass; core and web-ui tests
pass (119). The initial scoped provider/UI/core run passed 310 tests with 9 real
provider tests skipped; its two boundary failures were resolved by the browser
package registration and passed in the scoped rerun. The gallery build required
refreshing the local utils build artifact to expose its current source exports;
the production build then passed.

## Open items (deferred, with their milestone)

- tinyjs CI, toolchain pinning, size and cold-start assertions (owner:
  not now).
- tinyjs, awaiting a proposal: tee writes synchronous on the loop thread;
  the TLS configuration rebuilt per request.
- tinybash reference suites (just-bash compat, oils spec, GNU tests) and
  the split-equivalence test (M11, needs auto-provision).
- CI re-deriving the corpus goldens from bash (`TINYBASH_CHECK_GOLDENS=1`
  on the Linux job), with the tinyjs CI.
- A file named like an option matched by a glob in a builtin's operands
  reaches a flag the whitelist excludes only at run time (see the
  namespace entry above); reported as an error, not upgraded.
- Execution evidence beyond the dispatch record: a runner job surviving a
  socket drop and re-attached by id from its tee files, and the guest
  reconcile re-adopting a running VM instead of killing and saving it. Both
  change deliberate designs (`runner.md` § Disconnect semantics,
  `managed-hosts.md`); until then a command whose process died has an
  unknown outcome by definition (`sessions-and-targets.md` § Recovery).

## M13 — web package removal

Removed the `packages/web` scaffold and its workspace, command, and boundary graph entries at the user’s request. `web-ui` and `web-gallery` remain. Validation: lockfile refreshed; `bun test --conditions development packages/core` passed all 30 tests, including workspace scripts and package boundary coverage.


## M13 — Three-part delivery plan (2026-09-05)

Status: planned; M13.1 is next. No application implementation in this checkpoint.

Owner decision: split the web milestone into frontend prototype development
(M13.1), incremental feature design and development in the prototype (M13.2),
and complete frontend/backend integration (M13.3). The first two parts do not
require backend integration. The existing gallery and web-ui are available;
the application package is absent following the requested scaffold removal.

Conclusion: prototype review determines the frontend experience and accepted
additions. The M12 API freeze must not block that work; identify required
contract changes during feature design and implement them in M13.3. Gallery
features such as pinning, unread state and extensions have not been approved
merely by splitting the milestone. Navigation, workspace semantics, preference
ownership, upload behavior and the visual baseline remain prototype design
questions, not prerequisites for backend integration now.

Updated roadmap acceptance and verification coverage for all three parts,
product delivery guidance and the backend API baseline wording. Removed stale
product text about retaining a web-demo. Verification: documentation diff and
cross-record consistency review; no runtime code changed or tests run.


### M13.1 — Application prototype, first checkpoint (2026-09-05)

Status: implementation in progress on `codex/web-prototype`.

Owner direction: start the prototype in `packages/web`; select the stack first.
The stack follows the product record and existing browser packages: Vue 3,
TypeScript, Vite, vue-router, Pinia, Tailwind 4 and `web-ui`. The application
uses in-memory fixtures and a scripted simulation clock; no backend or model
connection. Refresh restores fixture state; appearance uses web-ui's theme store.

The first checkpoint covers the app frame, shared gallery sidebar, conversation
tabs, scripted streaming with queue/stop/retry, archive/restore, project selection
and creation, attached-device selection, attachments, settings and a demo entry
screen. Sidebar presentation moves into `web-ui`; fixture data stays in gallery.
Pinning, deletion and extensions remain outside this checkpoint's product UI.

Owner feedback: compressed HTML and Vue source was not acceptable. Format the
entry HTML and source as ordinary readable multiline code before verification.

Owner review identified a more significant mistake: the initial application
invented controls and layout despite the gallery baseline. Removed the custom
tabs, message list, composer, dialogs and generic control CSS. The app now uses
shared AgentTabItem, AgentMessageList, SessionSurface/SessionDock and UI controls.
Extracted SessionComposer so gallery, application and AgentMessageInput share
the input presentation. Sidebar presentation also has one implementation in
web-ui. The app retains domain state and wiring only.

Browser review exposed an empty attachment slot affecting capsule geometry,
optional boolean props masking draft state, and nested dropdowns dismissing
their parent dialog. Fixed those in shared components, using the existing
overlay family mechanism for dialog children. Checked desktop, 390px layout,
multiline input, light/dark appearance and nested settings menus.

Remaining M13.1 coverage: full setup/login/password/admin forms, directory browsing
and creation, provider vendor/custom/subscription forms, detailed usage, attached
host rename, interactive command stdin, steering and final baseline interaction
review. These need prototype work before M13.1 acceptance; this checkpoint does
not claim the milestone complete. M13.2 additions are still undecided.

Verification: all three browser package typechecks and 124 scoped core/web-ui/web
tests passed. Application and gallery production builds passed (both report the
existing large-chunk warning); browser walkthroughs cover
scripted send/recovery, navigation and the shared controls above. No real-model
tests. This checkpoint remains partial M13.1 coverage pending the listed flows
and owner review of the corrected baseline.

### M13.1 — Sidebar navigation and text interaction (2026-09-05)

Owner decisions: remove the application's conversation tabs; section labels and
interface controls must not select text; selected conversation titles keep normal
weight; long gallery input must not display a horizontal scrollbar.

Removed the app tab component, tab state and open/close wiring. Sidebar selection
and the route own navigation. Shared sidebar, menu headings, tooltips, semantic
controls, application dialog headings and gallery chrome now prevent text
selection. Inputs explicitly retain selection, as do transcript content and the
workspace path. Removed the selected-row font-weight override.

The single-line shared composer used overflow-x:auto, which exposed a scrollbar.
It now clips the scrollbar while retaining caret scrolling; expanded input wraps
long unbroken strings and scrolls vertically. Browser checks confirm zero app
tabs, 400 weight for all conversation rows, non-selectable sidebar headings,
selectable message content/input/path, 960-character single-line caret scrolling,
and a 1200-character unbroken multiline value with no horizontal overflow.

Validation: all three browser typechecks, 94 scoped web/web-ui tests and the web
production build passed. The build retains the existing large-chunk warning.
M13.1 remains in progress; this checkpoint addresses the owner's interaction review.

### M13.1 — Shared sidebar polish (2026-09-05)

Owner review requested simultaneous marquee edge fades, the conversation title
with device information on its right, hollow idle status rings, lighter-looking
type, no account chevron, archive/composer transitions, a project-header create
button and pin/archive hover actions. The owner rejected unnecessarily reducing
the gallery's available sidebar interactions in the application.

Changed the shared sidebar implementation so gallery and application receive
the same row, project and account presentation. Restored application pin handling;
kept context menus and multi-selection for rename/move and other existing actions.
Removed the empty-project create row. Both marquee masks now appear only during
marquee playback, instead of the right edge preceding the left. Idle rows show
a hollow ring; active/error indicators remain. Header and archive transition
changes belong to the application conversation view.

Typography inspection found normal 400-weight conversation text and explicit
font-smoothing:auto in shared CSS. Changed macOS rendering to grayscale
antialiasing rather than reducing text below normal weight.

Verification: three browser typechecks, 94 scoped web/web-ui tests and the web
production build passed (existing large-chunk warning). Browser checks cover
pin/unpin, archive/restore, project-header creation and updated header/sidebar
appearance. M13.1 remains in progress.

### M13.1 — Workspace metadata in the header (2026-09-06)

Owner request: show useful context at the top right, including device, branch
and workspace name. Added a target-owned WorkspaceInfo assembly using shared
Button/Tooltip primitives, device status, workspace name and nullable branch
metadata. Clicking opens the existing target picker; the tooltip carries the
full path. Branches remain fixture data, not a read of local Git state. New
workspaces explicitly display unavailable branch metadata.

Verification: browser typechecks and production build passed. Browser review
confirmed metadata to the right of the title, target picker opening, and a
390px viewport with metadata below the title and no horizontal page overflow.
M13.1 remains in progress.

### M13.1 — Stable projects and dropdown dismissal (2026-09-06)

Owner corrections: creating conversations must not reorder projects, outside
clicks dismiss dropdowns, and hostless headers omit environment metadata.

Project grouping preserves the supplied project list order; conversation sorting
continues within each group. Removed the redundant group activity timestamp.
Dialog still supplies overlay ownership to child menus, but its own surface is
not registered as a menu panel: including it caused clicks elsewhere in the
dialog to be ignored by dropdown outside-click handling. Hostless views omit
WorkspaceInfo, whose contract now requires a project.

Coverage: sidebar/__tests__/group-conversations.test.ts verifies stable group
order despite newer/pinned conversations, activity changes and empty projects.
Browser checks confirmed notes creation leaves demi before notes, outside-click
dismissal inside a dialog and on the page, and the hostless header's absence of
environment metadata. Scoped web/web-ui tests, browser typechecks and production
build are checked for this checkpoint. M13.1 remains in progress.

### M13.1 — Sidebar project spacing (2026-09-06)

Owner requested hostnames immediately after project names, the new-conversation
glyph instead of plus, tighter spacing to the fold chevron, and smaller idle
rings. Shared SidebarProjectHeader groups name/host with a 6px gap and groups
SquarePen/chevron without an extra flex gap. Idle rings are 8px with a 1px border;
the status column alignment and running/error indicators stay consistent.
Browser visual review and browser package typechecks passed. Both gallery and
application use these shared components. M13.1 remains in progress.

### M13.1 — Project hover background (2026-09-06)

Owner requested no background on project-item hover. Removed the shared project
row's hover/menu background classes and retained its keyboard focus ring.
This applies to gallery and application. Verified the scoped presentation diff;
no behavior or state changes. M13.1 remains in progress.

### M13.1 — Independent workspace controls (2026-09-06)

Owner request: separate device dropdown, workspace file browser and searchable
branch selection/creation. Replaced the combined header action with three shared
Button/Dropdown assemblies. Added a target-owned file-browser dialog with fixture
directories, parent navigation, filtering, file preview and folder selection.
Device changes select a directory on that device before moving only the current
conversation; existing workspace records are reused by device/path. Offline
devices and running/archived conversation switches are blocked.

Branch data lives on the prototype workspace. Search supports existing selection,
creation and invalid/duplicate-name feedback; the menu focuses the search input
when opened. New workspaces start with no branch. No real Git or filesystem calls
occur. Fixture files and in-memory branch behavior are labeled in their surfaces.

Verification: browser typechecks, six scoped web tests and production build passed
(existing large-chunk warning). Browser walkthrough confirmed branch creation and
header update, file preview, Cloud device/folder selection, and a long branch name
at 390px without horizontal page overflow. Branch-name coverage is documented in
web-prototype.md. M13.1 remains in progress.

### M13.1 — Project hover affordances and host icons (2026-09-06)

Owner requested removal of idle rings and the sidebar prototype caption; project
fold chevrons replace the left folder only on hover, and create buttons appear
on hover. Cloud uses an icon instead of its name; devices use computer/name.
Updated the shared project row with same-size stacked folder/chevron glyphs and
hover-only create visibility (keyboard focus also exposes the actionable button).
Kept active/error status marks and removed idle rings. Added explicit hostKind
to shared project presentation data and all its producers, avoiding display-name
inference. Cloud/device metadata uses flex centering in sidebar and header.

Verification: browser hover/leave checks confirmed icon opacity swaps and create
visibility, fold interaction worked, and Cloud screenshot review confirmed icon
alignment. Browser typechecks, 96 scoped web/web-ui tests and web production build
passed (existing large-chunk warning). M13.1 remains in progress.

### M13.1 — Main and attached host interaction (2026-09-06)

Owner requested an interaction design for one main execution host plus multiple
attached hosts, with binding entry points in the device menu. Verified the
contract in sessions-and-targets.md and product.md before implementation.
Standalone design: host-menu.md. HostMenu groups main environment, attached hosts,
searchable attachment and new-device registration. Attachment rows expose cwd,
online state, promotion, alias rename and detach. Hostless keeps a management
icon without inventing main-device metadata. Archived bindings are read-only.

Replaced ID-only prototype attachments with device/name/cwd rows. Switching main
hosts removes the incoming attachment and retains the departed host's cwd;
binding starts at home, deduplicates devices and creates unique aliases. Revoking
a prototype device removes its bindings. Removed the separate checkbox binding
UI from TargetDialog so host management has one entry point.

Verification: eight scoped web tests cover main/attached identity, cwd retention,
duplicate binding and alias collisions alongside existing conversation behavior.
Browser walkthrough passed attach-offline, count update, submenu inspection,
rename, detach and Hostless menu access. Browser typechecks and web build passed
with the existing chunk-size warning. M13.1 remains in progress.

### M13.1 — Sidebar motion and focused host menu (2026-09-06)

Owner changes: right-align project devices, crossfade device/create and
folder/chevron on hover, start marquees promptly with coordinated masks, and
remove directories, renaming and help copy from the host menu.

Project controls share grid positions and 150ms opacity transitions without
changing title width. Reduced-motion preferences disable these transitions.
Marquees measure after an 80ms width transition (100ms delay), start moving at
the first keyframe instead of holding for 12% of the total animation, and use
one registered opacity property for both mask edges. Removed the prototype host
rename action/editor and its tests; generated unique names remain internal state.
Host-menu design and prototype documentation reflect the focused device actions.

Verification: browser inspection confirmed early marquee movement, identical edge
alpha, 150ms device/create fades and a host menu without paths or help paragraphs.
Browser typechecks, scoped web/web-ui tests and production build passed. Restarted
the independent preview service after branch switching interrupted its watcher;
the review endpoint returns HTTP 200. M13.1 remains in progress.

### M13.1 — Recent directory menu (2026-09-06)

Owner requested recent directories before the full browser. The directory control
opens a menu of up to eight recent workspace paths on the current device, marks
the current path, and places Choose another directory at the bottom. Selecting
a recent path switches directly; choosing another opens the existing browser.
Recency is separate from project order and updates when using a workspace.

Verification: prototype/resources.test.ts checks recency deduplication without
reordering sidebar projects. Nine scoped web tests, browser typechecks and build
passed. Browser walkthrough confirmed direct switching, recency promotion and the
bottom action opening the current directory. M13.1 remains in progress.

### M13.1 — Product copy cleanup and read-only branch metadata (2026-09-06)

- Completed a product-surface audit across menus, workspace/project dialogs, account/settings, login, archive and notices. Removed instructional and simulation commentary and the Prototype settings section; retained field labels, actual status and validation errors.
- Current branch is read-only icon/text. Removed branch switching, creation, branch-list state and the obsolete branch validation module/test. Archive sits directly beside the conversation title.
- Verified 98 scoped web/web-ui tests, all three browser package typechecks and production build. Browser walkthrough confirmed title/archive placement, static branch metadata and the cleaned directory chooser. Existing large-chunk build warning remains.
- Branch checkout interrupted the development watcher; restarted the independent launchctl service and verified the acceptance page at port 18934. Product UI should not carry developer guidance; simulation constraints remain in this document and the prototype design record.

### M13.1 — Shared menu row conventions (2026-09-06)

- Main host is a single icon/label/value/submenu row. Attached hosts use the shared section title and right-aligned online/offline dots; removed custom text panels and repeated identity headings. Empty attachment groups are omitted.
- Added value and indicator presentation to shared MenuItem. ModelMenu, the project picker and gallery examples use the same label/value contract. Gallery includes the host section/status pattern. Product containers retain only menu dimensions and domain state.
- Verified 98 scoped web/web-ui tests, all browser package typechecks and production build. Browser checks covered main-host submenu, attachment selection, gray-to-green status changes (6px shared dot), click-outside dismissal and model-menu label/value rendering. No business-state change or additional unit tests were needed for this presentation change.
- The branch checkout interrupted the Vite watcher; restarted the independent background service for acceptance at port 18934.

### M13.1 — Attached-host status in the icon gutter (2026-09-06)

- Moved the shared MenuItem status dot into the left icon gutter, replacing the attached device glyph. The right side keeps the submenu arrow. Updated the gallery specimen and host-menu design.
- Verified all browser package typechecks, production build and browser rendering with an attached device. Status color and accessible label are preserved. No new tests for this presentation-only adjustment.
- Fast-forwarded the new requirement branch before switching the checkout, keeping the independent acceptance server running throughout.

### M13.1 — Unified host selection menus (2026-09-06)

- Main host and Attach device share HostPicker. Main selection shows Cloud and Connect new device above the search field and device results; attachment selection omits Cloud. Neither picker lists Hostless as a binding target.
- Shared Menu owns the fixed header slot and separators. Its disabled-item predicate covers pointer and keyboard selection, so offline/current main devices remain visible without being selectable. Existing attachment filtering excludes the current main and already-bound devices.
- Updated gallery specimens and the host-menu design. Verified 98 scoped web/web-ui tests, all browser typechecks and web build; repeated gallery typecheck after correcting a duplicate icon import in its specimen. Browser checks covered both picker layouts, filtering, persistent header actions and blocked keyboard selection of an offline main host.
- Acceptance service remained running throughout the branch switch and checks.

### M13.1 — Usable demo host switching and complete device lists (2026-09-06)

- Both seeded devices start online so switching is immediately reviewable. Attachment selection shows every owned device, disabling already-bound devices rather than removing them.
- Fixed Menu's explicit iconless override and let HostPicker reserve its icon gutter. Empty item lists therefore cannot hide the fixed Connect new device plus icon.
- Verified browser switching from zan-mbp to build-01 through folder selection, complete attachment lists when all devices are bound, and the fixed header icon with empty search results. Browser typechecks, 98 scoped web/web-ui tests and production build passed.

### M13.1 — Sidebar marquee hover delay (2026-09-06)

- Sidebar title scrolling starts after a 500ms hover delay, shared by the application and gallery. Existing scroll speed and edge fades are unchanged.
- Verified the shared web-ui typecheck. No new tests for this timing constant adjustment.

### M13.1 — Sidebar section heading spacing (2026-09-06)

- Added 6px below both Conversations and Projects headings in the shared sidebar, separating titles from the first item in the application and gallery.
- Shared web-ui typecheck passed. No new tests for this spacing-only change.

### M13.1 — Project create-button hit target (2026-09-06)

- Reproduced the fading hostname intercepting the create button's center hit target even at opacity zero. Made the decorative host layer pointer-transparent so clicks reach the overlaid new-conversation button.
- Browser verification created conversations from both collapsed and expanded project rows; the route changed and the project fold state was preserved. Shared web-ui typecheck passed. The fix is shared with gallery and retains the crossfade.

### M13.1 — Project folder icons without hover chevrons (2026-09-06)

- Removed project hover chevrons and their crossfade. Folder icons remain visible and reflect the fold state; clicking the row still folds/unfolds the project.
- Shared sidebar change applies to app and gallery. Web-ui typecheck passed; no new tests for the glyph-only adjustment.
