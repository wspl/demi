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
  (the managed-host hand-over is M10). `createBackend` picks it for
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
- The hostless → managed-host hand-over on `outside` is M10; today the
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
  `version`/`abi` check, `tinyjsc`, and tinyjs running `runtime`
  commands in command mode. Two findings stay open for a proposal first:
  tee writes on the loop thread, the TLS configuration rebuilt per
  request.
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

### Open items (deferred, with their milestone)

- tinyjs CI, toolchain pinning, size and cold-start assertions (owner:
  not now).
- tinyjs, awaiting a proposal: tee writes synchronous on the loop thread;
  the TLS configuration rebuilt per request.
- The loader's runner side (M9): a directory `ManifestSource`, a module
  import strategy other than `blob:`, the manifest served to the runner.
- `@demicodes/host-virtual` reduced to the store-backed Host and its
  spawn refusal deleted (M9, with just-bash).
- `@demicodes/host-local` and the local open-box assembly deleted (M9,
  after the M1 and M4 suites pass on the new runner).
- tinybash reference suites (just-bash compat, oils spec, GNU tests) and
  the split-equivalence test (M10, needs auto-provision).
- CI re-deriving the corpus goldens from bash (`TINYBASH_CHECK_GOLDENS=1`
  on the Linux job), with the tinyjs CI.
- A file named like an option matched by a glob in a builtin's operands
  reaches a flag the whitelist excludes only at run time (see the
  namespace entry above); reported as an error, not upgraded.
