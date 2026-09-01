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

## M4 — Runner productized

Status: **done** (2026-09-01; `f758090` control-plane device/workspace
records, `012d41b` runner management module, `e9ae6c9` remote-Host
resolution + acceptance, `9d0499d` CLI output polish). Repo-wide typecheck
green; backend/runner/runner-protocol suites 24 tests green.

What landed:

- **Pairing spec finalized before implementation** (`0bfdaf5`, `cee9feb`):
  claim codes are 128-bit random Crockford base32, single-use, expiring,
  claim endpoint rate-limited per user; the end-to-end flow diagram lives
  in demi-next.md § Connection model.
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

## Host design round: user/managed hosts, `demi host`, lifecycle (2026-09-01)

Design-only round (no code); the outcome is the "Execution targets: user
hosts and managed hosts" section in `docs/demi-next.md`, the M6 rewrite,
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
