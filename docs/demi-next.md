# Demi Next: Multi-User Web

| | |
|---|---|
| Date | 2026-08-31 |
| Status | Proposed (design; provider/CLI facts verified against current code and local mocks) |
| Scope | The hosted multi-user chat product: backend (conversation, LLM, runner-management, credential, accounting modules), runner program, virtual execution, session/host model, roadmap |

## Motivation

A deployable, multi-user, pure-web chat GUI built on Demi. Differentiators
over ChatGPT/Claude web UIs:

- **BYOK and subscription reuse**: users bring API keys or connect their
  existing subscriptions (Claude Code, Codex, Grok, …); all credentials are
  stored server-side and usable from any of the user's devices.
- **Choice of execution environment**: agent tools run on the user's own
  devices via the runner program, in operator-managed containers, or in a
  zero-setup virtual environment.
- **Chat-first default**: most sessions are conversation with light tools and
  need no real machine at all.

## Protocol layering (the core shape)

```
web  ←— our protocol —→  backend  ←— official provider wires —→  LLM providers
                            │
                            └←— our runner protocol (Host RPC) —→ runner (user device / container)
                                     └─ claude code CLI (spawned by backend, runs on runner)
                                            └←— Anthropic wire, passing through backend —→ Claude backend
```

- Browser ↔ backend: Demi's agent protocol (`ClientFrame`/`ServerFrame`) plus
  the Web API (everything the page calls that is not the per-conversation
  agent socket) — designed from scratch, inheriting nothing from
  `@demicodes/web`'s demo endpoints.
- Backend ↔ LLM providers: the official wire protocols, spoken by the real
  provider runtimes (`createAnthropicApiProvider`, `createCodexProvider`, …)
  instantiated **inside the backend** with vault credentials at their native
  endpoints.
- Backend ↔ runner: Demi's runner protocol — a remote form of the `Host`
  contract (filesystem ops, process spawn with streamed stdio). See the
  Runner program section.
- The one special case is the **Claude Code provider**: its transport is the
  CLI, which must run on a real machine. The provider itself runs in the
  backend like every other provider, but spawns its CLI process on the
  session's runner through the runner protocol's ordinary `spawn`, speaking
  stream-json over the spawned process's stdio. The CLI's own HTTPS traffic
  goes to the Claude backend **through a backend passthrough endpoint**
  (`ANTHROPIC_BASE_URL` + runner token as `CLAUDE_CODE_OAUTH_TOKEN`, swapped
  for the vault token at the passthrough) so it is metered and credentialed
  like everything else.

## Invariants

1. **Sessions live in the backend.** AgentSession, the transcript, tool
   orchestration, and the bash interpreter all run in the backend; the
   authoritative conversation store is backend-local. Runners hold no
   conversation state.
2. **The execution target is a mutable session property.** A session's tools
   execute against a `Host`: the in-process virtual Host, a runner's remote
   Host, or an operator container's remote Host. Demi's agent was built for
   this — `AgentHarness.host` resolves a stable Host per execution target from
   action metadata, with per-Host BashEnvironment reuse. Switching targets is
   a first-class operation at a turn boundary, announced to the model with an
   injected context block.
3. **All model traffic flows through the backend, and all credentials live
   server-side.** HTTP provider runtimes run in the backend with vault
   credentials; the Claude Code CLI's Anthropic-wire traffic passes through
   the backend with token swap. Runners hold zero credentials.

## Components

### Backend (`@demicodes/backend`, product leaf)

One program, one architecture. Scaling is running more copies of the same
program, never splitting it into roles (serverless hosting stays out of
scope so its constraints cannot leak into the interfaces):

```
Self-host (one instance):

  browser ─────────┐
                   ├──►  demi-backend  ──►  SQLite (one file)
  runner ──────────┘     (complete: Web API + sessions + vault + runner mgmt)


Scaled (same program × N):

                        ┌──►  demi-backend #1 (everything for users a,b) ──┐
  browsers ──►  router  ├──►  demi-backend #2 (everything for users c,d) ──┼──►  Postgres (shared)
  runners  ──►  (pins   ├──►  demi-backend #3 (everything for users e,f) ──┘
                by user) └──►  …
```

Every instance is a **complete** backend for its assigned users — user x's
HTTP, conversation sockets, runner socket, virtual Hosts, and CLI processes
are all pinned to instance `hash(x)`. The affinity is natural because
conversations, devices, and (isolated mode) connections are all user-owned,
so nothing stateful ever crosses instances. The "sessions have exactly one
home" invariant is unchanged — only the number of homes grows. Self-host is
the N=1 degenerate form with no router at all. v1 milestones implement N=1
only.

How the routing works — the router is an off-the-shelf reverse proxy
(nginx-class) doing consistent-hash upstream selection; we develop no
routing code and ship only a sample config:

- **The routing key**: at login the backend sets a uid cookie (browser
  traffic); at claim time the backend hands the runner its owner's route key
  alongside the device token, and the runner sends it as a header on every
  reconnect (device traffic). Same key ⇒ same hash ⇒ browser and devices of
  one user converge on one instance. The proxy holds no state and knows no
  business — it is config (the static server list) plus arithmetic over the
  key.
- **Hashing happens at connection establishment only.** Established
  WebSockets are never rerouted mid-stream: during a scale event, in-flight
  turns finish on the old instance; only new connections land on the new
  mapping.
- **Scale events move ~1/N of users** (consistent hashing), and a moved
  user's experience is exactly the already-defined backend-restart
  semantics: in-flight turn interrupted only if the connection happened to
  drop mid-turn, zero history loss (checkpoints in the shared database, any
  instance can restore any user), runner auto-reconnects to the new home.
  Scale events are rare, operator-initiated maintenance.
- **The one N>1 correctness mechanism: checkpoint write fencing.** During a
  remap a stale session object may linger on the old instance while the new
  home restores from checkpoint; checkpoint rows carry an epoch so a stale
  instance's write is rejected by the database. A few lines of SQL
  constraint, implemented when the scaled topology lands — not in v1.

Spoken of as modules, not separate services:

- **Conversation module**: AgentServer/AgentSession hosting for every session,
  transcript persistence in the backend store, session index, cold-history
  reads (served to the same web-ui components — e.g. as a full-sync
  `transcript_reset` frame or an equivalent read endpoint; follow mainstream
  practice, no second rendering path), compaction, session concurrency via the
  existing client-owned session ids + `SessionOwnershipRegistry` takeover
  (`packages/agent/src/server.ts:182`).
- **LLM module**: the provider runtimes assembled from the credential vault;
  the Anthropic passthrough endpoint used solely by Claude Code CLI processes
  (authenticates the runner token in `Authorization`, swaps in the vault OAuth
  token); model catalog and quota surfaces for the web UI.
- **Runner management module**: device registry (claim tokens, device tokens,
  online status = socket state), the runner-protocol server, and per-session
  Host handles over connected runners.
- **Credential vault module**: BYOK keys and subscription OAuth tokens,
  the providers' device-login flows, token refresh — including the hard-coded
  auth endpoints (`auth.openai.com`, `auth.x.ai`, `console.anthropic.com`),
  which are only ever called from here.
- **Usage accounting module**: ledger aggregated from `TokenUsage` the LLM
  module observes firsthand (`user × session × provider × model`);
  enforcement (rate limits, over-quota refusal) at the inference entry points.
  There is no trust gap — runners never self-report usage.
- **Auth module**: users, web login, device claiming. The data model is
  multi-user from the first milestone (userId on every row); the login surface
  can arrive later, tenant-shaped data cannot.

### Runner (`@demicodes/runner`, product leaf)

The program users install on their devices — detailed in the Runner program
section below. The same binary in a container image is the **docker runner**
(operator-managed): a hosting variant, not a new code path.

### Virtual execution (default entry)

`@demicodes/host-virtual`: a platform-neutral `Host` fs whose bytes live in a
pluggable blob backend, mirroring the database's two topologies: **local
filesystem directory** (N=1 default, full fs semantics) or **S3-compatible
object storage** (required at N>1 — a remapped user's virtual files must be
reachable from any instance). Namespace is per-conversation. The S3 backend
maps paths to keys (listing via prefix, rename as copy+delete) and cleanly
fails the exotic operations object storage cannot express (symlink/link/
chmod); quotas are two hardcoded numbers (per-file and per-conversation
caps), and archived conversations keep their files — the no-deletion
principle applies. (`Host.store` is not its concern: the backend composes
every Host it hands the harness — virtual or remote — with the backend
store, uniformly.)

**Code execution on virtual: JS via a WASM-sandboxed interpreter; Python
explicitly not offered.** A `node`/`js` command is registered into the
CommandRegistry of virtual-target shells only (per-Host command composition —
it must never shadow the real `node` on device workspaces), executing scripts
in a QuickJS-compiled-to-WASM interpreter: fs wired to the virtual `Host.fs`,
hard memory/time limits, no network, capability-based imports only. Chosen
over embedding V8 isolates deliberately: hostile code is interpreted, never
JIT-compiled, so the escape surface is a WASM-runtime bug rather than the
recurring JIT-CVE class — and no half-maintained native V8 bridge enters the
credential-holding process. The 10–50× interpreter slowdown is irrelevant at
chat-script scale. This ships as an independent branch after M1, preceded by
its own small design record (runtime choice, limits, console mapping). The
`executable_not_found` spawn contract is unaffected — registered commands
dispatch before spawn.
Its `process.spawn` must resolve with `spawnError.kind = 'executable_not_found'`
— the portable-command fallback engages only on that error kind
(`packages/shell/src/portable-commands.ts`), so anything else would break even
`cat` — and the shell surfaces an actionable "virtual environment — upgrade
to a device to run real programs" message. Sessions default to it: zero setup,
chat and portable-command tool use work immediately (`BashEnvironment` routes
portable commands through `Host.fs` with zero spawns —
`packages/shell/src/__tests__/environment.test.ts:1485`), and because the
session runs in the backend anyway, there is no extra lifecycle machinery.
Limits surface honestly: `bash`, `sh`, `sleep`, background jobs, and any real
binary require a real target (`packages/shell/src/portable-commands.ts:18`),
and the Claude Code provider needs a process-capable target for its CLI —
gated by a provider capability flag, never by hard-coded provider names.

### Web frontend (`@demicodes/web`, product leaf)

A separate frontend package — the product shell as a pure SPA (no SSR: a
logged-in application with no SEO surface): Vue 3 + Vite, vue-router for
pages (login, chat, devices, connections, usage, admin), **Pinia** for app
state, consuming `@demicodes/web-ui` (unchanged: injected `AgentClient` +
transport-agnostic control client) and the Web API. Production: the built
assets ship inside the backend image and `@demicodes/backend` serves them
alongside `/api`; development: Vite dev server proxying `/api`.

Naming: the existing dev-only `@demicodes/web` product is renamed
`web-demo` when the new package is scaffolded (M1), lives on as a deprecated
demo, and gets deleted once the product covers it.

Layout and information architecture:

- Classic three-pane: sidebar (conversation list + new-conversation + user
  menu at the bottom), chat area, and a conversation header carrying the
  title, execution-target display/switch, and the conversation-level
  operations from the "everything Demi implements gets exposed" rule
  (compact, abort, retry, …). The **model picker lives at the input area —
  it is web-ui's existing design**; the shell only feeds it the catalog
  grouped by connection.
- The conversation list is **grouped by workspace**: the first group is
  always the no-workspace conversations (virtual is not a workspace),
  followed by one group per workspace. Plus an archived view entry.
- **Settings are modal dialogs**, opened from the sidebar user menu, with
  tabs: devices, connections, usage, user management (admin), instance
  settings (admin). No settings routes.
- Responsive (sidebar collapses to a drawer on mobile — watching and
  steering server-side turns from a phone is part of the product story); no
  PWA, no offline, no push.
- The chat body itself is zero design burden: message rendering, tool
  blocks, markdown, streaming, input, and the model picker are existing
  web-ui components; the shell builds the frame above, the settings dialogs,
  and the target picker (device list + directory browse).

## Session and host model

- Every session is an AgentSession in the backend. Its current **execution
  target** is a session property; the harness resolves the Host per action
  from it. A target is either `virtual` or a **workspace** — a lightweight
  named entity, `(device, path, name)`. A workspace is only an attribute of
  conversations: conversations move freely between workspaces (and to/from
  virtual), and nothing else hangs off it — no per-workspace settings or
  permissions.
- **Target switching is one generic, unconstrained mechanism.** Any target →
  any target, in any direction, at a turn boundary; the product adds no
  restrictions. The machine's job is minimal: re-resolve the Host and inject
  a context block stating the previous and new target/directory and that
  files stay where they were. The single mechanical extra is switching **out
  of virtual**: the virtual files are written to a temp directory on the new
  target (`/tmp/demi-migration-<id>/`) — otherwise they would be permanently
  unreachable — and the context block names that path so **the model
  relocates what it needs itself**. No code-side placement, merging, or
  conflict rules. When the new target is on the same device, the context
  block notes the old directory is still directly accessible. Because
  command artifacts are real
  files on the target, the context block also covers "full outputs of earlier
  commands live on the previous target" — artifact paths the model saw before
  the switch are stale on the new Host. The runtime side is already safe:
  per-Host environments plus cross-Host shell-handle ownership checks fence
  old handles, and new commands write artifacts under the new target's
  `commandArtifactsDir` naturally.
- **Runner offline** mid-turn: in-flight spawns die and fs calls fail; these
  surface as ordinary tool errors and the turn continues or ends — the session
  itself is never lost. Offline targets leave the session fully readable and
  chattable (switch to virtual), just unable to touch that machine.
- **Persistence** splits along the existing contract line: conversation state
  (checkpoints, subagent records) goes through `Host.store` and is
  backend-local; **command artifacts are real files on the execution target**
  (since the `/@` virtual-fs replacement, `Host.commandArtifactsDir` lives in
  the fs namespace), so full command output follows the target and is
  unavailable while that runner is offline — the transcript's bounded tool
  views in the checkpoint remain always readable. The `journal.jsonl`
  incremental design from `docs/session-storage-and-naming.md` remains a
  storage optimization, no longer a network necessity.
- **Browser refresh / disconnect** is inherently safe: turns run server-side;
  the client reattaches with `open` + `sync_transcript`
  (`packages/agent/src/client.ts:200`). Verify during implementation that a
  binding close never aborts an in-flight turn.

## Product design

The functional design of the product itself — what the backend modules
actually store and expose, and what the web UI consists of.

### Instance mode: shared vs isolated

An instance runs in exactly one of two opposing modes; there is no mixing and
no per-connection ownership machinery:

- **Shared**: provider connections are instance-wide. Only users with the
  admin role may create/modify/delete them; ordinary users cannot touch any
  provider configuration — they just use the models. Typical self-host.
- **Isolated**: every user manages their own provider connections; nothing is
  shared between users. Typical public host.

Usage is metered per user in both modes. The mode is the only instance
setting. A **provider connection** is
an API key or a completed subscription login; since either mode allows
multiple connections of the same provider type, model selection is keyed by
`(connectionId, modelId)` — the backend instantiates one provider runtime per
connection and uses the connectionId as its providerId.

### User system (minimal final state)

Username + password (modern hash), cookie session (httpOnly, sliding
expiry); the conversation stream WebSocket and the Web API authenticate by
the same
same-origin cookie. **No self-registration and no password recovery of any
kind** — zero mail/SMTP dependency. Accounts are managed entirely from an
admin page:

- **master**: the instance's first account, created at initial setup. Can do
  everything, including creating admins.
- **admin**: everything master can do except creating admins — creates users,
  resets passwords, manages the instance's provider connections (shared
  mode), edits instance settings.
- **user**: uses the product.

No organizations, teams, or further roles.

### Conversation system

A conversation is one AgentSession plus one metadata row: id, owner userId,
title, execution target, provider/model selection, created/updated
timestamps, archived flag.

- **Archive only, no delete.** Archiving hides a conversation from the
  default list; an archived view lists them and any can be restored. No data
  is ever deleted in v1 — no deletion semantics to design, no artifact
  cleanup problem.
- **Titles**: default is the first user message (Demi's existing behavior) +
  manual rename. LLM-generated auto-titles are planned for later, not v1.
- **New conversation is one click**: immediately typeable — target defaults
  to virtual, model defaults to the user's last-used selection (first
  available connection's default on first use). Both are session properties
  changeable at any time.
- **Message-level operations: everything Demi implements gets exposed** — a
  standing rule, never curtail an implemented capability. Concretely, from
  the frame protocol: mid-turn steering, the message queue
  (queue/dequeue/edit-queued/clear), abort, retry, resume, manual compaction,
  mid-conversation provider/model switch (`set_provider`), and interactive
  stdin to running commands (`shell_write`). Most of this ships with the
  web-ui components already.

The **streaming interface is the agent frame protocol** — no parallel SSE
API; cold history rides the same rendering path (full-sync frame).

### Attachments — two distinct flows

- **Message attachments** (model-visible media, the closed media set in
  `@demicodes/core`; the picker filters by the selected model's
  `acceptedExtensions`): uploaded via **HTTP POST → attachment id**; the
  `send` frame carries only a small reference block, and the conversation
  module resolves references into inline bytes before handing the message to
  the AgentSession — providers still receive inline bytes, zero provider
  changes. Never inline large binaries into the frame socket: WS messages
  serialize, so a slow multi-MB upload would block steer/abort/ping exactly
  when the user most needs them (and every mainstream LLM frontend uses
  upload-then-reference for the same reason). This is a minimal
  implementation of the `source.ref` blob design already planned in
  `docs/session-storage-and-naming.md`. Size cap hardcoded (single number,
  configurable later). Arbitrary-file message attachments are a future item
  (requires new core block types — designed then, not now).
- **Workspace files** (files the agent should work on; anything non-media
  dropped into the chat routes here automatically): written into the
  execution target's working directory via the backend (browser → HTTP upload
  → Host RPC `writeFile`), with the file path auto-inserted into the input as
  a text reference. Filesystem data, not conversation data.

### Provider management (web)

The connections page (admin-only in shared mode, per-user in isolated mode):
paste an API key (openai / anthropic / google, with optional
compatible-endpoint baseUrl), or connect a subscription — the backend runs
the provider's device-login flow and the UI shows the verification code/URL
and polls until claimed. Configuring a connection makes **all of its models**
usable — model lists come live from the provider runtime's catalog and are
never stored, with one exception: compatible-endpoint connections take a
user-entered model id list (part of the connection config) plus a **Test**
button that fires one cheap request to validate the endpoint/key. Each
connection shows auth state and the latest quota snapshot; connections can be
deleted. There is no model-level configuration of any kind — no per-model
enablement, aliases, or parameter overrides. Model pickers are fed from the
backend's aggregated catalog, grouped by connection.

### Web UI surface inventory

Chat view (existing web-ui components) + conversation sidebar; model/provider
picker; execution-target picker (device list + directory browser via Host RPC
`readdir`); device management (claim-token entry, online status, revoke);
connections page (above); usage page (ledger, per user); admin-only user
management (create user, reset password, grant admin — master only) and
instance settings. Nothing else in the first final state — sharing, collaboration, and
search are explicitly out.

### Database

Two dialects are final state, matching the two topologies: **SQLite**
(`bun:sqlite`, one file, WAL) for N=1 — zero dependencies, backup is copying
a file — and **Postgres** for N>1, where the shared database is what the
instances have in common. **No ORM, no query builder**: a hand-rolled thin
storage module with hand-written SQL kept to the two dialects' common subset,
two drivers (`bun:sqlite` / a postgres client), per-statement dialect
handling only where unavoidable, and numbered `.sql` migration files. All
persistent concerns (users, conversation index, connections/vault, ledger,
the DB-backed `HostStore`) are structured data; transactions in either
dialect provide the `writeJson` atomicity the checkpoint path relies on. v1
runs SQLite only. Command artifacts stay on execution targets.

Schema (final state, no speculative columns):

```
users            id, username, password_hash, role(master|admin|user), created_at
web_sessions     token_hash, user_id, expires_at
conversations    id, user_id, title, archived, workspace_id(NULL = virtual),
                 connection_id, model_id, created_at, updated_at
workspaces       id, user_id, device_id, path, name, created_at
devices          id, user_id, name, platform, token_hash, claimed_at, last_seen_at
connections      id, owner_user_id(NULL in shared mode), type, label,
                 config(encrypted), created_at
usage_ledger     id, user_id, conversation_id, connection_id, model_id,
                 input_tokens, output_tokens, cache_tokens…, created_at
attachments      id, user_id, media_type, bytes(BLOB), created_at
host_store       scope, key, value_json          ← the DB HostStore (checkpoints)
settings         key, value                       ← instance mode only
```

Notes: pending claim tokens live in memory (an unclaimed runner socket holds
them; a backend restart just reprints); device online status is runtime
state, `last_seen_at` is display-only; the transcript is inside the
checkpoint, not a table. **Credential encryption**: `connections.config` is
encrypted at rest with an instance secret (generated into the data directory
on first start; a shared secret config across instances at N>1) — cheap
protection against the database file leaking alone (backups, copies), with
no KMS or per-user key machinery. **Ledger granularity**: one raw row per
provider request as `TokenUsage` events arrive (a turn may produce several);
aggregation happens at query time, never at write time.

### Web API (browser ↔ backend)

Designed from scratch — nothing is inherited from `@demicodes/web`'s demo
`/control` WS RPC or its `/agent?cwd=` addressing, both of which do not
survive. The product API has exactly two kinds of traffic:

1. **Application data — plain HTTP REST.** Auth, conversations metadata,
   devices, connections, models, usage, users, settings, uploads: all
   resource CRUD. Cookie auth, standard status codes, cacheable reads,
   upload progress for free. Errors: HTTP status + `{code, message}` body,
   codes as stable strings. No `/v1` prefix — frontend and backend ship
   together in a self-hosted instance; version negotiation is speculative.
2. **The live conversation stream — one WebSocket per open conversation**,
   `WS /api/conversations/:id/stream`, carrying Demi's agent frame protocol
   (`ClientFrame`/`ServerFrame` — a real contract of `@demicodes/agent`, not
   a demo artifact). The execution target/cwd is resolved server-side from
   the conversation record; the browser never names a cwd.

`@demicodes/web-ui` is unaffected: it consumes a transport-agnostic client
interface, which the product shell backs with fetch instead of the demo WS
RPC. No server push in v1 — pages poll on open and on an interval.

Resource layout (concrete scope fed into the roadmap):

| Resource | Endpoints | Lands in |
|---|---|---|
| auth | `POST /api/auth/login`, `POST /api/auth/logout`, `GET /api/auth/me` | M1 stub → M5 real |
| conversations | `GET/POST /api/conversations`; `PATCH /api/conversations/:id` (rename/archive/unarchive/target/model); `GET /api/conversations/:id/transcript` (cold history); `WS /api/conversations/:id/stream` | M1 |
| models | `GET /api/models` (aggregated catalog, grouped by connection) | M1 |
| devices | `GET /api/devices`, `POST /api/devices/claim`, `DELETE /api/devices/:id`, `GET /api/devices/:id/fs?path=…` (directory browse) | M2 |
| workspaces | `GET/POST /api/workspaces`, `PATCH/DELETE /api/workspaces/:id` (rename/remove the pointer; never touches files) | M4 |
| connections | `GET/POST /api/connections`, `DELETE /api/connections/:id`, `POST /api/connections/:id/test`, `POST /api/connections/subscription-login` + `GET …/subscription-login/:id` (poll) | M3 |
| usage | `GET /api/usage` | M3 |
| attachments | `POST /api/attachments` (returns reference id), `POST /api/conversations/:id/workspace-files` | M4 |
| admin | `GET/POST/PATCH /api/users` (create, reset password; grant admin — master only), `GET/PUT /api/settings` (instance mode) | M5 |

## Runner program

The runner turns a user's machine into an **execution target**. It is not an
agent process: it contains no LLM logic of its own and runs no AgentSession.
Conceptually it is a remote implementation of Demi's `Host` contract
(`packages/shell/src/host.ts`) — it exposes this machine's filesystem and
process execution to the backend over Demi's own protocol, and nothing else.
Even the Claude Code CLI (the one provider whose transport must live on a real
machine) reaches the runner as an ordinary remote `spawn` issued by the
backend-side provider — the runner has no claude-specific code.

Design principle: no speculative constraints. No workspace restrictions, no
local policy layer, no configuration beyond what the connection needs.

### Responsibilities

1. **Device identity and connection.** On first start, connect to the backend,
   receive a claim token, and print it; the user enters that token in the web
   UI to attach the device to their account permanently. The backend then
   pushes a device token over the same socket, which the runner persists and
   uses for all subsequent connects. One outbound WebSocket, exponential
   backoff on reconnect; no inbound ports, ever. Device online status in the
   web UI is simply this socket's state.
2. **Serving the Host contract.** Answer filesystem operations and process
   spawns from the backend, scoped per request to a working directory the
   session names. Any existing directory is a valid workspace; an operation on
   a nonexistent path fails with an error, nothing more. Spawned processes
   stream stdin/stdout/stderr over the socket and honor kill signals
   (`HostSpawnHandle` semantics, platform-neutral).
That is the whole surface. There is deliberately no runner-side control RPC:
directory picking for the web UI is ordinary `readdir` over the same Host RPC,
recently-used workspaces come from the backend's own session index, and
version/platform ride the `hello`.

Non-responsibilities: user authentication (backend-only), credentials of any
kind (backend vault — the Claude Code CLI's env token is the backend-issued
runner token, swapped for the real credential at the backend passthrough),
transcript or checkpoint storage (backend-only — the runner stores nothing
about conversations), shell interpretation (just-bash runs in the backend
against this Host), and provider logic (the CLI it may be asked to spawn is
driven entirely by the backend-side claude-code provider over spawn stdio).

### Process shape and local state

Single binary, one command: `demi-runner run [--backend <url>]`. First start
prints the claim token and waits for the claim; later starts authenticate with
the persisted device token. Service installation comes with the packaging
milestone.

Dependency footprint is deliberately thin: `@demicodes/shell` (Host contract),
`@demicodes/host-local` (`LocalHost`), `@demicodes/utils`, and the shared
runner-protocol package. No agent, coding-agent, or provider packages.

```
~/.demi/
  runner.json          # backend URL, device id
  runner-token         # device token (0600)
```

### Connection model

One outbound multiplexed WebSocket. Runner presence equals the state of this
single socket, so the backend's device-online flag and any in-flight
operations follow it atomically. Messages are portable JSON
(`@demicodes/utils` codec — `Uint8Array`/`bigint` round-trip, which file bytes
need).

Claim/auth handshake:

| Direction | Message | Purpose |
|---|---|---|
| r → b | `hello { deviceToken?, runner: { name, platform, version, identity: { uid, gid, hostname } } }` | authenticate (token absent on an unclaimed first start); `identity` because `HostIdentity` is read synchronously at shell creation, so the proxy Host must have it before it is handed out |
| b → r | `hello_ok { deviceId }` | accepted (claimed device) |
| b → r | `claim_pending { claimToken }` | unclaimed: runner prints the token, keeps the socket open |
| b → r | `claimed { deviceToken }` | user entered the token in the web UI; runner persists and is live |
| b → r | `hello_error { reason }` | rejected (revoked device, bad token) |
| both | `ping` / `pong` | liveness (backend-driven interval) |

Host RPC — the wire form of the `Host` contract's `fs` and `process` facets
(`Host.store` never crosses this protocol; conversation state is
backend-local):

| Direction | Message | Purpose |
|---|---|---|
| b → r | `fs_call { id, op, args }` | one filesystem operation (the `HostFileSystem` method set) |
| r → b | `fs_result { id, ok, result \| error }` | its result |
| b → r | `spawn { spawnId, command, args, cwd, env, … }` | start a process |
| r → b | `spawn_output { spawnId, stream, bytes }` | stdout/stderr chunks |
| b → r | `spawn_stdin { spawnId, bytes }` / `spawn_kill { spawnId, signal }` | input / termination |
| r → b | `spawn_exit { spawnId, code, signal? }` | process finished |

The exact `fs_call` op set mirrors `HostFileSystem` (18 async, plain-data
methods; `Uint8Array`/`Date` ride the portable JSON codec) and is fixed when
the protocol package lands. Proxy-side notes from the Host portability audit:

- **Cwd**: the proxy Host uses `createLogicalHostCwd`
  (`packages/shell/src/host.ts:130`) — the contract's own path-string fallback
  for Hosts that cannot hold a directory fd. `LocalHostCwd`'s
  `/proc/self/fd/N` spawn anchors and sync `snapshot().restore()` never cross
  the wire.
- **Spawn streams**: the runner pushes one ordered, stream-tagged chunk
  sequence per spawn; the proxy derives the handle's `stdout`/`stderr`/merged
  `output` views from it (three independent iterables must not double-count).
- **Artifacts**: `Host.commandArtifactsDir` is part of the fs namespace —
  command artifact files live on the execution target and are written through
  `fs_call` like any other file.
- Known cost to accept: fs operations are per-op round trips, and since the
  backend always runs in a datacenter while runners sit on user devices, this
  is the product's normal case, not an edge. The merged scan-routing change
  helps a lot — `cat`/`grep`/`rg`/`find`/`ls` are `preferHostSpawn` and run
  as a single remote spawn on real runners. Known hot spots with planned
  fixes: PATH resolution (one `stat` per PATH entry per command — proxy-side
  cache), the stat+read pair per file read, and the 3–4 sequential artifact
  writes per command status change (pipeline, don't block command
  completion). Target after those: 2–3 RTTs per command.

**Considered and rejected: running the agent loop on the runner.** It would
buy zero-latency tool execution, at the price of putting the wire through the
system's fastest-moving interface instead of its most stable one. The Host
contract (fs + spawn) is essentially frozen; the agent internals (inference
items, transcript patches, subagents, compaction) change constantly — and the
runner binary on user devices is the hardest component to update, so it must
contain the least-changing code. Runner-side loops also resurrect everything
this design deleted: transcript sync back to the backend, a browser↔runner
frame relay, sessions with two homes (lease, migration machinery, offline
lock-in), and a runner fattened with agent/coding-agent/provider deps. The
latency win is bounded (turn wall-clock is inference-dominated); the costs
are structural, and the latency cost is bounded and optimizable — so the
decision is closed, not parked behind a measurement.

Disconnect semantics: when the socket drops, in-flight spawns on the runner
are killed and pending fs calls fail; the backend surfaces the failure into
the affected turns as ordinary tool errors (the session itself lives in the
backend and is not lost). On reconnect the device is simply online again.

Provider catalog, model listing, credential state, and quota all live in the
backend; the web UI never asks the runner about anything — it asks the
backend, which uses this same Host RPC when it needs to look at the device
(e.g. browsing directories for the workspace picker).

### Trust model

- The device token authorizes "this device executes for this user" and
  carries no credentials of any other kind.
- The runner trusts the backend for user identity: operations arriving on its
  socket belong to the claiming user, and the runner performs them in whatever
  directory they name. Multi-user sharing of one runner is out of scope.

## Claude Code specifics (the special case, contained)

- `packages/provider-claude-code` spawns its CLI with `child_process.spawn`
  today (`transport.ts:65`) and builds env from `process.env` (`cli.ts:36`).
  Two additive options make it remote-capable: **injectable spawn** (a
  `Host.process`-shaped spawn function) and a **public env overlay**
  (`ANTHROPIC_BASE_URL`, `CLAUDE_CODE_OAUTH_TOKEN`).
- Verified with local mocks (CLI 2.1.220): with `ANTHROPIC_BASE_URL` set, the
  CLI sends exactly one request class — `POST /v1/messages?beta=true` — and
  adopts the env token in its `Authorization` header; fed a minimal SSE
  stream it completes the turn. One best-effort direct
  `CONNECT api.anthropic.com:443` ignores the base URL, survives refusal, and
  is not load-bearing. The passthrough endpoint therefore needs to serve only
  the Messages wire.
- Transcript replay needs no CLI-side state: `--no-session-persistence`, no
  session ids, plain-message replay (`jsonl.ts:79`) — so target switching
  works for Claude Code sessions like any other; the next turn cold-starts a
  CLI on the new target.
- The provider-side `/api/oauth/usage` quota probe and OAuth refresh become
  vault concerns; the models.dev catalog fetch runs in the backend.

## Changes to existing packages (all additive)

1. `@demicodes/provider-claude-code` — injectable spawn + public CLI env
   overlay option.
2. `@demicodes/provider` — an execution-requirement capability flag in
   provider metadata (claude-code declares it; virtual targets refuse it with
   upgrade guidance).
3. New packages: `@demicodes/backend` (product leaf), `@demicodes/web` (the
   frontend product leaf; the old dev product is renamed `web-demo`),
   `@demicodes/runner`
   (product leaf), a small platform-neutral runner-protocol package (envelope,
   Host RPC types), and `@demicodes/host-virtual` (platform-neutral fs
   semantics over an injected blob backend; the local-dir and S3 backend
   implementations live in `@demicodes/backend`).
4. `@demicodes/agent` — optional, later: `journal.jsonl` incremental
   persistence (planned remaining work in
   `docs/session-storage-and-naming.md`) as a storage optimization.

Dropped from earlier drafts (superseded by this architecture): the frame
relay, per-provider proxy-mode `baseUrl`/headers options, codex/grok header
fixes, any "external auth" provider mode, the remote-inference RPC, checkpoint
write-through, and the session lease layer (backend-internal
`SessionOwnershipRegistry` suffices — sessions have exactly one home).

`docs/package-boundaries.md` gains registry entries for the new packages when
implementation starts. The existing single-user Vite-dev product is renamed
`web-demo` (deprecated, deleted once the product covers it); the new frontend
product takes the `@demicodes/web` name.

## Prior art and the empty quadrant

Every component of this architecture has large-scale precedent; only the
combination is rare.

- "Agent loop in a service, execution environment across a wire" is the
  standard cloud-sandbox agent shape (E2B/Modal/Daytona-style sandbox APIs are
  fs + exec over HTTP; Devin, Codex cloud, Copilot coding agent all run this
  way).
- "Orchestration in the cloud, execution on user-owned machines" is the CI
  self-hosted-runner shape (GitHub Actions runners; Ansible control nodes).
- Command-granular remote execution — our main path after `preferHostSpawn` —
  is SSH-shaped: one round trip per command plus streamed output, proven over
  WAN for decades.

The genuinely unoccupied quadrant is the combination: **loop in a datacenter +
execution target on user devices + a fine-grained fs protocol**. It is empty
for two reasons, in this order of importance:

1. **Trust asymmetry (structural, the main reason).** A datacenter service
   holding "execute arbitrary commands on user devices" means a backend
   compromise turns every claimed device into a bot. This is why Claude
   Code's Remote Control chose the opposite placement (loop on the device,
   cloud as UI relay). We accept the asymmetry deliberately, with three
   answers: self-host-first positioning (the user controls the datacenter),
   a runner so thin and frozen it is auditable (fs + spawn + claim, nothing
   else), and explicit device claiming. If this product ever becomes public
   multi-tenant SaaS, this becomes the number-one design pressure, and the
   likely response is device-side capability narrowing (per-directory or
   per-session grants) — the direction of the deleted workspace allowlist.
   Not building that now is correct for the self-host context; the line
   exists and we know where it is.
2. **Fine-grained fs over WAN has a famous failure (engineering, the lesser
   reason).** VS Code Remote originally tried "editor logic local, files
   remote," failed on per-op latency, and moved the extension host to the
   file side. The differences here: our load is agent-turn-granular, not
   human-keystroke-granular, and the heavy operations are already
   command-granular via scan routing; the chatty residue is bounded and has
   named optimizations (PATH cache, artifact pipelining).

The category itself — "drive my own machine from a hosted web UI" — is
months old; an unoccupied quadrant in a young category is not a verdict.
Where others hesitate, this design places explicit answers: command-granular
routing plus named optimizations on the latency risk, and self-hosting plus a
thin frozen runner on the trust risk.

## Verified facts (2026-08-31, code reading + local mocks only)

Retained from the design's verification passes; no real provider endpoint was
contacted (a local deny-proxy caught escape attempts).

- Every HTTP provider runtime accepts `baseUrl` + extra headers and its full
  endpoint surface follows `baseUrl` (inference, catalogs, quota probes);
  auth-plane endpoints are hard-coded — fine, they are vault-only. API-key
  options on openai/anthropic/google are resolver functions.
- Codex: WS transport is a scheme swap on the same host/path; its auth relies
  on Bun's non-standard `WebSocket(url, {headers})` (works in the backend
  runtime); `x-codex-*` quota rides on inference response headers, observed
  firsthand by the runtime.
- Claude Code CLI: see the specifics section above (single request class via
  base-URL override; env-token adoption; harmless direct CONNECT attempt).
- `Host` is platform-neutral and enforced (`packages/shell/src/__tests__/root-entry.test.ts:15`);
  portable commands run without spawns; in-memory Host/store shapes exist as
  test doubles (`packages/coding-agent/src/__tests__/coding-harness.test.ts:256`).
- `AgentSession.fromCheckpoint` restores from `{transcript, state, …}` and
  force-completes executing tool calls (`packages/agent/src/session.ts:106`);
  session ids are client-owned with takeover semantics
  (`packages/agent/src/server.ts:398`).

### Host portability audit (current code, post-0.19.3)

- The `Host` contract is wire-ready: all `HostFileSystem` methods are async
  with plain-data inputs/outputs (`HostFileStat` is de-Node-ified; the
  portable JSON codec covers `Uint8Array`/`Date`), spawn params/exit are
  plain data, `kill` takes a string signal by boundary rule. The two
  non-wire-safe spots have designed answers: `HostCwd` fd anchors → the
  contract's own `createLogicalHostCwd` fallback; sync `Host.identity` →
  carried in the runner `hello`.
- **Multi-Host-per-session is an existing, tested mechanism**:
  `AgentHarness.host(action metadata)` with per-Host `BashEnvironment` reuse
  and cross-Host shell-handle ownership checks
  (`packages/agent/src/__tests__/host-routing.test.ts`). Target switching
  builds on it; only the product harness (today fixed-host in
  `coding-agent`) needs a routing implementation in the backend. Action
  metadata is not checkpointed — the target comes from the backend session
  registry per action, as designed.
- **No remote Host / RPC exists anywhere in the repo** — the runner protocol
  is greenfield, as planned.
- Scan routing (`preferHostSpawn`) makes real-runner remoting cheap for the
  heavy commands (single spawn instead of per-file fs calls); the chatty
  residue (PATH resolution, globs, redirections, `test`, artifact writes) is
  listed in the Runner program section as measure-first hot spots.
- The DB-backed `HostStore` must re-provide the atomicity `LocalHostStore`
  now guarantees for `writeJson` (callers rely on it since the
  atomic-checkpoint change) and should serve `list` + bulk reads efficiently
  (`listConversations` is read-per-key today).

### Storage pluggability (audited: DB-backed backend needs no interface changes)

- Conversation state is fully behind `HostStore` (4 methods) — checkpoints,
  subagent records, future blobs/journal (command artifacts are fs files on
  the execution target, not store entries). The backend implements a DB-backed
  `HostStore` and composes it into every Host it hands the harness; the agent
  layer is untouched.
- Credentials are fully behind injectable auth stores: every subscription
  provider creator accepts `authStore?:`
  (`packages/provider-codex/src/provider.ts:52`,
  `packages/provider-claude-code/src/provider.ts:34`,
  `packages/provider-grok-build/src/provider.ts:33`), each a two-method
  interface (`status()` + `resolveAuth`/`resolveAccess`) with refresh and
  persistence as implementation concerns; injection takes precedence over the
  file/pool stores. API-key providers take resolver functions. Device-login
  flows return token material without persisting
  (`runCodexDeviceLogin(): CodexAuthDotJson`), so the vault stores the return
  value. The vault is therefore three authStore implementations plus one
  `HostStore` implementation, all inside `@demicodes/backend`.
- The file-based implementations (`File*AuthStore`,
  `@demicodes/provider/credentials-pool`, `~/.demi` layout) stay for the
  local products and the runner's own machine-local state; no migration or
  compatibility layer.
- Remaining hard-wired filesystem touches: the claude-code transport's
  `statSync`/`child_process.spawn` (covered by the M0 injectable-spawn item),
  the claude-code tmpdir wire log (disabled via `DEMI_CLAUDE_WIRE_LOG=0` in
  the backend), and grok's optional `version.json` read (constant fallback,
  harmless).

## Implementation roadmap

Ordering principles: the riskiest long-lived contracts first (runner protocol,
host-per-action resolution, vault), every milestone ends runnable, product
surface last. Each item is its own branch off `main`.

**M0 — Groundwork (independent small branches, parallelizable)**
- claude-code injectable spawn + env overlay options.
- provider capability flag.
- Integration test: one AgentSession whose harness switches Host targets
  between turns (two temp-dir LocalHosts) with an injected context block —
  the migration primitive in miniature.

**M1 — Two parallel tracks (no dependency between them)**

*Track A — Backend skeleton + virtual default (first demoable node).*
`@demicodes/backend` serving the conversation stream + the Web API
multi-user-shaped (stub
user), conversation persistence in the backend store, session index,
`host-virtual` as the default target. Providers are **operator-assembled**
exactly like `@demicodes/web` today (env keys / operator logins) — the vault
is explicitly out of M1 scope. Accept: zero-setup browser chat with
portable-command tools; refresh mid-turn reattaches to the running turn;
cold history readable.

*Track B — Runner protocol core.* The runner-protocol package (fs RPC,
streaming spawn, handshake) and the `demi-runner` binary, exercised against a
**bare AgentServer** in tests — no product integration, no device registry
yet. Runs in parallel because it depends on nothing in Track A and is the
design's only greenfield contract — the earlier it exists, the earlier its
implementation problems surface.

**M2 — Runner productized**
Claim-by-token flow, device registry with online status, backend harness
host resolution to remote Hosts. Accept: a session executes real shell
commands and file edits on a claimed device; runner disconnect surfaces as
tool errors without losing the session; reconnect resumes.

**M3 — LLM module + credential vault + metering + Claude Code**
Two acceptance steps in order:
1. *BYOK + metering* (no dependencies): vault key storage, per-user provider
   assembly for the API-key providers, usage ledger + enforcement. This is
   the product's minimum viable form — a user pastes a key and chats — and
   stands alone as a demoable point.
2. *Subscriptions + Claude Code* (depends on M2): provider device-login
   flows + refresh in the vault, the Anthropic passthrough, claude-code
   sessions spawning their CLI on the session's runner. Accept: turns with
   every provider against mock LLM endpoints, runner holding zero
   credentials; CLI chain end-to-end through the passthrough (skip when no
   `claude` binary); real-subscription smoke manual only, never an ungated
   test.

**M4 — Target switching + attachments**
Turn-boundary switching UI + context injection + the out-of-virtual tmp-dump
(model relocates); workspaces CRUD; offline-target degradation (read/chat via
virtual);
message-attachment upload (inline media content blocks) and workspace file
drop (Host RPC `writeFile`). Accept: switch, upgrade, and offline flows each
covered by integration tests; an uploaded image round-trips through a
StubProvider turn and the checkpoint.

**M5 — Multi-user product shell**
Real auth (username/password, cookie sessions, master/admin/user roles, no
registration, no recovery), user-management admin page, device management UI,
connections/usage/instance-settings pages, session list / target picker,
tenant-isolation authz matrix. UI deliberately late: earlier
milestones accept with the stub user and existing web-ui surfaces.

**M6 — Deployment packaging**
Docker images for runner and backend; end-to-end acceptance. Pure hosting
work after interfaces are frozen.

Independent branch, any time after M1: the virtual-target JS command
(WASM-sandboxed QuickJS; own design record first).

Deliberately deferred: journal.jsonl storage optimization, fs RPC
batching/caching (only when measurements demand), per-wire usage
reconciliation.

## Milestone verification

Three tiers, matching repo conventions: (1) model-free automated tests
(StubProvider + local mocks, scoped `bun test packages/<pkg>`, CI-gating);
(2) env-gated real-credential smoke (`real-*.e2e.test.ts`, `DEMI_*_E2E`,
manual pre-release only — real models are never a merge gate); (3) manual
checklists only for UI look-and-feel and packaging smoke.

| M | Verification |
|---|---|
| M0 | Spawn-injection + `buildClaudeEnv` overlay assertions (no CLI). Capability-flag tests. Host-switch integration: StubProvider session runs turn 1 against LocalHost A, turn 2 against LocalHost B; assert context block injected, per-Host BashEnvironment isolation, transcript continuity. |
| M1-A | Backend integration in one test process: browser-side `AgentClient` (in-process transport) + virtual-Host session; detach client mid-turn with a slow StubProvider → turn completes → reattach sees the full result (covers refresh-immunity and binding-close-must-not-abort); cold-history read equals live transcript; portable commands work, spawn fails with the upgrade message. |
| M1-B | Protocol codec round-trips (portable JSON incl. `Uint8Array`). Remote-Host integration against a bare AgentServer: session executes `cat`/`tee`/spawn on a runner in a temp dir; kill the runner mid-command → tool error, session continues; reconnect → next command succeeds. |
| M2 | Claim-flow integration (unclaimed → claim → reconnect with device token; bad/revoked token; claim-token expiry). Backend host routing to a claimed device's remote Host; device online status follows the socket. |
| M3 | Step 1: vault key storage + per-user assembly unit tests; ledger aggregation from StubProvider usage. Step 2: login-flow state machines against mock auth endpoints + refresh; passthrough mock upstream asserts token swap and single request class; claude-code-on-runner chain with the real CLI against a mock upstream, skipped when no `claude` binary. Tier 2: one gated real-subscription smoke per provider. |
| M4 | Switch integration, all directions unconstrained: real→real (files stay + honest context block; same-device note when applicable), virtual→real (files land in the target tmp dir, context block names the path, no code-side placement), real→virtual (fresh virtual fs + context block), mid-turn switch refused, concurrent switch has one winner; offline target → session readable and chattable on virtual. |
| M5 | Tenant-isolation authz matrix (every API action by user A against user B's data asserts denial); device revoke + re-claim through the management UI. Tier 3: UI manual checklist. |
| M6 | Tier 3 scripted smoke: build both images, claim a containerized runner against a local backend, run one full turn end-to-end; optional CI stage. |

Test modules and their coverage get documented per milestone as they land, per
the repo's design-record rules.

## Open questions

- Whether the web UI should offer directory creation on a device, or that
  stays a local action.
- fs RPC batching threshold — measure first (see the Runner program section).
