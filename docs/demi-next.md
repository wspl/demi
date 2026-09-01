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

- Browser ↔ backend: Demi's agent protocol (`ClientFrame`/`ServerFrame`) on
  the per-conversation stream socket, plus the Web API — plain HTTP REST for
  everything else the page calls (see Product design).
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
program plus one internal control-plane process (serverless hosting stays
out of scope so its constraints cannot leak into the interfaces):

```
Self-host (one instance):

  browser ─────────┐
                   ├──►  demi-backend  ──►  control.sqlite
  runner ──────────┘     (complete: Web API │  + conversations/<id>.sqlite (per conv)
                          + sessions + vault│  + blob dir
                          + runner mgmt)    └─ (litestream ──► S3, optional)


Scaled (same program × N + one internal control-plane service):

                        ┌──►  demi-backend #1 (users a,b) ─► own conversations/*.sqlite ─┐
  browsers ──►  router  ├──►  demi-backend #2 (users c,d) ─► own conversations/*.sqlite ─┼─► demi-controld
  runners  ──►  (pins   ├──►  demi-backend #3 (users e,f) ─► own conversations/*.sqlite ─┘   (internal RPC,
                by user) └──►  …                                every node: litestream ─► S3   control.sqlite)
```

Every instance is a **complete** backend for its assigned users — user x's
HTTP, conversation sockets, runner socket, virtual Hosts, and CLI processes
are all pinned to the instance the user→worker map names. The affinity is
natural because
conversations, devices, and (isolated mode) connections are all user-owned,
so nothing stateful ever crosses instances. The "sessions have exactly one
home" invariant is unchanged — only the number of homes grows. Self-host is
the N=1 degenerate form with no router at all. v1 milestones implement N=1
only.

How the routing works — the router is an off-the-shelf reverse proxy
(nginx-class) selecting the upstream from the routing key; we develop no
routing code and ship only a sample config:

- **The routing key**: at login the backend sets a uid cookie (browser
  traffic); at claim time the backend hands the runner its owner's route key
  alongside the device token, and the runner sends it as a header on every
  reconnect (device traffic). Same key ⇒ same map entry ⇒ browser and
  devices of one user converge on one instance. The proxy holds no state
  and knows no business — it is config (the static user→worker map over a
  static server list) plus a lookup on the key. Requests without a key yet
  (login, registration) may land on any worker; those endpoints only talk
  to the control plane, so every worker answers them identically.
- **Routing happens at connection establishment only.** Established
  WebSockets are never rerouted mid-stream: during a scale event, in-flight
  turns finish on the old instance; only new connections land on the new
  mapping.
- **Scale events move users explicitly**, and a moved user's experience is
  exactly a backend restart: stop the user's sessions on the source
  instance, `litestream restore` their conversation files on the target,
  update the routing map; the runner auto-reconnects to the new home.
  Conversation data lives only on the owning instance (replicated to S3),
  so there is no shared mutable state to fence — a stale instance has
  nothing it could write to. Scale events are rare, operator-initiated
  maintenance.
- **Control-plane access is the only cross-instance traffic**: workers
  reach `demi-controld` through the internal `ControlService` RPC (see
  Database below). Everything user-request-shaped stays on the user's
  worker.

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
principle applies. `Host.store` is not its concern: the backend composes
every Host it hands the harness — virtual or remote — with the backend
store, uniformly.

Sessions default to the virtual target: zero setup, chat and
portable-command tool use work immediately (`BashEnvironment` routes portable
commands through `Host.fs` with zero spawns —
`packages/shell/src/__tests__/environment.test.ts:1485`), and because the
session runs in the backend anyway, there is no extra lifecycle machinery.
Its `process.spawn` must resolve with
`spawnError.kind = 'executable_not_found'` — the portable-command fallback
engages only on that error kind, so anything else would break even `cat` —
and the shell surfaces an actionable "virtual environment — upgrade to a
device to run real programs" message. Limits surface honestly: `bash`, `sh`,
`sleep`, background jobs, and any real binary require a real target
(`packages/shell/src/portable-commands.ts:18`), and the Claude Code provider
needs a process-capable target for its CLI — gated by a provider capability
flag, never by hard-coded provider names.

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
chat-script scale. Registered commands dispatch before spawn, so the
`executable_not_found` contract above is unaffected. Ships as an independent
branch after M2, preceded by its own small design record (runtime choice,
limits, console mapping).

### Web frontend (`@demicodes/web`, product leaf)

A separate frontend package — the product shell as a pure SPA (no SSR: a
logged-in application with no SEO surface): Vue 3 + Vite, vue-router for
pages (login, chat, devices, connections, usage, admin), **Pinia** for app
state, consuming `@demicodes/web-ui` (unchanged: injected `AgentClient` +
transport-agnostic control client) and the Web API. Production: the built
assets ship inside the backend image and `@demicodes/backend` serves them
alongside `/api`; development: Vite dev server proxying `/api`.

Naming: the existing dev-only `@demicodes/web` product is renamed
`web-demo` when the new package is scaffolded (M8 — the UI is its own
dedicated milestone, last among feature work, built against the frozen API
so completed functionality never forces UI rework), lives on as a deprecated
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
- **Component placement rule**: generic components and LLM-domain components
  all live in `@demicodes/web-ui` — it is the design system plus LLM
  component library (message rendering, tool blocks, input, model picker,
  and the dialogs/forms/pickers this product adds). `@demicodes/web` keeps
  only the application-frame containers (nav, sidebar, page scaffolding) and
  the wiring. web-ui stays product-neutral in its dependencies (data in via
  props/slots, injected clients); in-repo consumption is by workspace
  dependency (the registry-semver rule is for external consumers).
- Visual language follows web-ui's existing theme system (light/dark);
  **English-only copy in v1** — i18n is a separate later effort, no
  framework introduced now.

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
  views in the persisted transcript remain always readable. The journal
  (incremental transcript persistence, from
  `docs/session-storage-and-naming.md`) is **required design**, realized as
  block rows in the per-conversation database (see Database): streaming
  appends block rows; nothing ever rewrites the whole transcript.
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
  bytes go to the blob store (local dir at N=1, S3 at N>1 — the same two
  topologies as the virtual fs), never into the database — the
  `attachments` table holds metadata + content hash only; the
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
picker; execution-target picker (device list + directory browser and
directory creation via Host RPC
`readdir`); device management (claim-token entry, online status, revoke);
connections page (above); usage page (ledger, per user); admin-only user
management (create user, reset password, grant admin — master only) and
instance settings. Nothing else in the first final state — sharing, collaboration, and
search are explicitly out.

### Database

**SQLite is the only dialect, in both topologies** (review record:
`docs/demi-next-progress.md`). The storage split follows the
write-frequency line:

- **`control.sqlite`** — one per deployment, the control-plane data: users,
  auth, devices, conversation index, connections/vault, ledger, attachment
  metadata, settings. Low write rate (the hottest writer is one ledger row
  per provider request), read-heavy (auth check per request, absorbed by a
  short-TTL token cache).
- **`conversations/<id>.sqlite`** — one file per conversation, the data
  plane: the transcript as **one row per block** (the journal — this is the
  required design, not an optimization: streaming persists by appending
  block rows, never by rewriting a whole-checkpoint JSON), plus session
  state and that conversation's `host_store` scope. High write rate
  (roughly one small append per second per active conversation), but each
  file has exactly one writer and the files never contend with each other.
- **Blob store** — attachment bytes and transcript media (`source.ref`),
  content-addressed `blobs/<sha256>`: local directory at N=1, S3 at N>1.
  Bytes never enter any database.
- **[Litestream](https://litestream.io/)** watches the data directory
  (`dir` + glob + `watch`: dynamically created conversation files are
  picked up automatically) and continuously replicates every `*.sqlite` to
  S3. It is the durability and recovery story — asynchronous, loses at
  most about the last sync interval (~1 s) on node death; restore is
  snapshot + LTX replay with gap detection, point-in-time capable.
  Optional at N=1 (a bare instance still needs zero external services),
  required at N>1.

**No ORM, no query builder**: a hand-rolled thin storage module with
hand-written SQL and numbered migrations, written for SQLite alone.

Deployment topology (N>1). Workers are fully symmetric; the control plane
is a dedicated internal service:

```
 Browser / Runner
       │  external HTTP/WS (the full public API — every worker serves all of it)
       ▼
 ┌───────────┐
 │    LB     │  routes by uid (static user→worker map)
 └─────┬─────┘
       ├──────────────────────────────┬─────────────────────────┐
       ▼                              ▼                         ▼
 ┌─────────────────┐          ┌─────────────────┐       ┌─────────────────┐
 │  worker 1       │          │  worker 2       │       │  worker N       │
 │  (symmetric)    │          │  (symmetric)    │       │  (symmetric)    │
 │ conversation    │          │                 │       │                 │
 │ hot path:       │          │     (same)      │       │     (same)      │
 │  WS stream,     │          │                 │       │                 │
 │  cold transcript│          │                 │       │                 │
 │   │ block append│          │                 │       │                 │
 │   ▼             │          │                 │       │                 │
 │ conversations/  │          │ conversations/  │       │ conversations/  │
 │  <id>.sqlite ×n │          │  <id>.sqlite ×n │       │  <id>.sqlite ×n │
 │ (this worker's  │          │                 │       │                 │
 │  users only)    │          │                 │       │                 │
 │                 │          │                 │       │                 │
 │ RemoteControl-  │          │ RemoteControl-  │       │ RemoteControl-  │
 │ Service ────┐   │          │ Service ────┐   │       │ Service ────┐   │
 │ litestream ─┼─▶ S3         │ litestream ─┼─▶ S3      │ litestream ─┼─▶ S3
 └─────────────┼───┘          └─────────────┼───┘       └─────────────┼───┘
               │                            │                         │
               └──────────────┬─────────────┴─────────────────────────┘
                              │  internal RPC only (private network,
                              │  service-token auth, ControlService
                              │  domain methods 1:1 — no SQL on the wire,
                              ▼  no cross-call transactions)
                    ┌───────────────────────┐
                    │  demi-controld  (× 1) │  no public listener;
                    │  ControlService RPC   │  single instance by design
                    │   │ in-process SQL,   │  (SQLite single-writer);
                    │   ▼ local txns only   │  failover = restore
                    │  control.sqlite       │  control.sqlite from S3,
                    │  litestream ──▶ S3    │  start a new controld,
                    └───────────────────────┘  repoint workers

 S3:  litestream/…     continuous replication of every *.sqlite
      blobs/<sha256>   attachment bytes + transcript media (source.ref)
```

**N=1 (v1) is the same picture with the LB and the extra workers deleted:
one process = worker + controld fused.** `ControlService` is the in-process
implementation, no RPC, no internal listener; the data directory is
byte-identical. That homogeneity is a design requirement: the storage
shape never changes between topologies, only the process placement does.

Interface topology — every external endpoint by where its data lives:

```
 (a) control-plane endpoints — worker is a thin shell over one RPC call
   POST /api/auth/login ────── createSession ─────────▶ ┌─────────┐
   GET  /api/conversations ─── listConversations ─────▶ │ demi-   │──▶ control
   POST /api/devices/claim ─── claimDevice ───────────▶ │ controld│    .sqlite
   …(connections, usage, admin, settings likewise)      └─────────┘

 (b) auth check on EVERY authed request — RPC, blunted by a local cache
   any request ──▶ worker token cache (short TTL) ──miss──▶ resolveSession

 (c) conversation hot path — worker-LOCAL, never crosses the network
   WS /:id/stream ──▶ live session ──▶ conversations/<id>.sqlite (block append)
   GET /:id/transcript ─────────────▶ conversations/<id>.sqlite (cold read)
   VirtualHost fs ops ──────────────▶ workspace files (no db at all)

 (d) mixed endpoints — local work + independent control-plane appends
   WS stream, turn ends ─┬─▶ <id>.sqlite (blocks, local)
                         ├─▶ appendUsage ───────▶ controld (ledger row)
                         └─▶ touchConversation ─▶ controld (updated_at, title)
   POST /api/attachments ─┬─▶ blob store (bytes)
                          └─▶ putAttachmentMeta ▶ controld (metadata row)

 (e) runner WS — terminates on the worker; controld sees identity only
   claim ──▶ claimDevice (once) · hello ──▶ token cache / resolve
   fs_call / spawn streams ⇆ live sessions (worker-local only)
```

Invariants this topology enforces:

- The public API exists only on workers; `demi-controld` has no public
  endpoint. Workers never touch `control.sqlite`; controld never touches
  conversation files.
- The RPC surface is the `ControlService` interface mapped 1:1 (Hono +
  `POST /rpc/<method>`, plain JSON, domain errors as `{code, message}`
  rebuilt by the client). One call = one atomic operation; transactions
  never span calls, and SQL never crosses the wire. Type safety comes from
  `RemoteControlService implements ControlService`, not from route
  inference.
- Every high-frequency write is worker-local (c); every controld call in
  (a/b/d/e) is low-rate or cache-absorbed. This is the load equation the
  whole design rests on.
- No cross-database transactions exist anywhere: the (d) pairs are
  independent appends/updates with no invariant between them (a lost
  ledger row never corrupts a transcript, and vice versa).
- User→worker assignment is partitioned, not balanced per-request: a user
  is pinned to one worker; rebalancing = migrating users (stop writes on
  the source, `litestream restore` the user's conversation files on the
  target, update the map).

Naming: interface `ControlService`, implementations `LocalControlService`
(in-process SQL — the N=1 backend and controld itself) and
`RemoteControlService` (the workers' RPC client); process `demi-controld`;
database `control.sqlite`. "Control plane / data plane" are the layer
names in prose; no component is named after a plane, and the `*Store`
suffix stays reserved for storage backends (`HostStore` family).

Schema — `control.sqlite` (final state, no speculative columns):

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
attachments      id, user_id, media_type, size_bytes, sha256, created_at
                 ← metadata only; bytes live in the blob store
settings         key, value                       ← instance mode only
```

Schema — `conversations/<id>.sqlite` (shape owned by the agent
persistence contract; exact columns land with the block-row
implementation):

```
blocks           one row per transcript block, append-only during streaming
session state    checkpoint fields other than the transcript
host_store       scope, key, value_json  ← this conversation's scope only
```

Notes: pending claim tokens live in memory (an unclaimed runner socket holds
them; a backend restart just reprints); claim tokens are 128-bit random
values, single-use with an expiry, and the claim endpoint is rate-limited
per user; device online status is runtime
state, `last_seen_at` is display-only. **Credential encryption**:
`connections.config` is encrypted at rest with an instance secret
(generated into the data directory on first start; a shared secret config
across instances at N>1) — cheap protection against the database file
leaking alone (backups, copies), with no KMS or per-user key machinery.
**Ledger granularity**: one raw row per provider request as `TokenUsage`
events arrive (a turn may produce several); aggregation happens at query
time, never at write time.

Production precedents this design stands on — every piece has one; the
composition is ours, every part and every seam is industry-standard:
per-tenant SQLite files with a single owning process (Bluesky PDS:
per-account SQLite + service-level SQLite databases, WAL, LRU handle
cache), symmetric data nodes with a dedicated low-write metadata service
(HDFS NameNode, TiDB PD, Kafka controller, Kubernetes control plane), a
service exclusively owning its database behind a domain API (the standard
database-per-service pattern), an HTTP service fronting SQLite (Grafana,
Gitea, Headscale), user-sharded SQLite-per-shard control planes
(Tailscale: one SQLite + one exclusive Go process per shard, tenants
migrate between shards), and streaming SQLite replication to S3
(Litestream). Alternatives weighed and rejected during the review are
archived in `docs/demi-next-progress.md`.

### Web API (browser ↔ backend)

The product API has exactly two kinds of traffic (nothing is carried over
from `web-demo`'s `/control` WS RPC or `/agent?cwd=` addressing):

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
interface, which the product shell backs with fetch. No server push in v1 —
pages poll on open and on an interval.

Resource layout (concrete scope fed into the roadmap):

| Resource | Endpoints | Lands in |
|---|---|---|
| auth | `POST /api/auth/login`, `POST /api/auth/logout`, `GET /api/auth/me` | M2 stub → M7 real |
| conversations | `GET/POST /api/conversations`; `PATCH /api/conversations/:id` (rename/archive/unarchive/target/model); `GET /api/conversations/:id/transcript` (cold history); `WS /api/conversations/:id/stream` | M2 |
| models | `GET /api/models` (aggregated catalog, grouped by connection) | M2 |
| devices | `GET /api/devices`, `POST /api/devices/claim`, `DELETE /api/devices/:id`, `GET /api/devices/:id/fs?path=…` (directory browse), `POST /api/devices/:id/fs` (create directory) | M4 |
| workspaces | `GET/POST /api/workspaces`, `PATCH/DELETE /api/workspaces/:id` (rename/remove the pointer; never touches files) | M6 |
| connections | `GET/POST /api/connections`, `DELETE /api/connections/:id`, `POST /api/connections/:id/test`, `POST /api/connections/subscription-login` + `GET …/subscription-login/:id` (poll) | M5 |
| usage | `GET /api/usage` | M5 |
| attachments | `POST /api/attachments` (returns reference id), `POST /api/conversations/:id/workspace-files` | M6 |
| admin | `GET/POST/PATCH /api/users` (create, reset password; grant admin — master only), `GET/PUT /api/settings` (instance mode) | M7 |

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
   receive a claim token, and print it; the user pastes that token into the web
   UI to attach the device to their account permanently. The claim token is a
   128-bit random value (Crockford base32, grouped for copy-paste — e.g.
   `9Z7K-M3FV-TQ2X-8HJD-4WPN-C6`): guessing is infeasible by entropy alone,
   and the token is additionally single-use, expiring, and the claim endpoint
   is rate-limited per user. Users copy-paste it, so length costs nothing.
   The backend then pushes a device token over the same socket, which the
   runner persists and uses for all subsequent connects. One outbound
   WebSocket, exponential backoff on reconnect; no inbound ports, ever.
   Device online status in the web UI is simply this socket's state.
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

The pairing flow, end to end — two user steps (run the command, paste the
code), no third:

```
① First start (unclaimed)

User's device                                Backend
─────────────                                ───────
$ demi-runner run --backend https://demi.example.com
      │
      │  opens ONE outbound WebSocket (no inbound ports, ever)
      ├────────────────────────────────────────►│
      │  hello { deviceToken: absent,           │  no token
      │          runner: {name, platform,       │  ⇒ unclaimed device
      │          version, identity} }           │  ⇒ generate claim token
      │◄────────────────────────────────────────┤    (in memory only)
      │  claim_pending { claimToken }           │
      │                                         │
   prints:                                      │
   ┌─────────────────────────────────┐          │
   │  Pairing code:                  │          │  socket stays open,
   │    9Z7K-M3FV-TQ2X-8HJD-4WPN-C6  │          │  runner just waits
   │  Enter it in the web UI to      │          │
   │  link this device.              │          │
   └─────────────────────────────────┘          │

② User pastes the code in the web UI

Browser (logged in as alice)                 Backend
────────────────────────────                 ───────
Devices page → paste code                    lookup claim token
      ├────────────────────────────────────► │  ⇒ bind device to alice
      │  POST /api/devices/claim             │  ⇒ mint permanent deviceToken
      │                                      │
      │                              push down the SAME open socket:
      │                              ┌───────┴──────────────────────►  runner
      │                              │  claimed { deviceToken }
      │                              │
      │                              │       runner persists and is live:
      │                              │         ~/.demi/runner.json
      │◄─────────────────────────────┤         ~/.demi/runner-token (0600)
      │  device shows "online"       │  online = socket state, nothing else

③ Every later start (claimed)

$ demi-runner run
      │  hello { deviceToken: "…", runner: {…} }
      ├────────────────────────────────────────►│  token valid
      │◄────────────────────────────────────────┤
      │  hello_ok { deviceId }                  │  device online
      │                                         │
      │  from here the socket carries only      │
      │  Host RPC: fs_call / fs_result /        │
      │  spawn / spawn_output / spawn_exit,     │
      │  plus ping/pong liveness                │
```

The only failure branch: a revoked or invalid device token answers
`hello_error { reason }`; the runner prompts to pair again (delete the
local token, back to ①).

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
latency win is bounded (turn wall-clock is inference-dominated) while the
costs are structural, so the decision is closed.

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
4. `@demicodes/agent` — the persistence contract becomes append-block +
   save-state (the journal, required design — see Database and
   `docs/session-storage-and-naming.md`): streaming appends finished
   blocks; nothing rewrites the whole transcript. Lands in M3.

Explicitly **not** part of this design — each of these is unnecessary once
sessions live in the backend, and none should be reintroduced: a
browser↔runner frame relay, per-provider proxy-mode `baseUrl`/headers
options, any "external auth" provider mode, a normalized remote-inference
RPC, checkpoint write-through from runners, and a session lease layer
(backend-internal `SessionOwnershipRegistry` suffices — sessions have
exactly one home).

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

Established by code reading and local-mock experiments; no real provider
endpoint was contacted (a local deny-proxy caught escape attempts).

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
  registry per action.
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

### Storage pluggability (audited)

- Conversation state is fully behind `HostStore` (4 methods) — checkpoints,
  subagent records, future blobs (command artifacts are fs files on
  the execution target, not store entries). The backend implements a DB-backed
  `HostStore` and composes it into every Host it hands the harness. The one
  deliberate agent-layer change is M3's persistence contract
  (append-block + save-state); everything else plugs in behind the
  existing seams.
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
surface last. Implementation status, pitfalls, and conclusions are tracked
live per milestone in `docs/demi-next-progress.md`.

**M0 — Groundwork (independent small branches, parallelizable)**
- claude-code injectable spawn + env overlay options.
- provider capability flag.
- Integration test: one AgentSession whose harness switches Host targets
  between turns (two temp-dir LocalHosts) with an injected context block —
  the migration primitive in miniature.

**M1 — Runner protocol core**
The runner-protocol package (fs RPC, streaming spawn, handshake) and the
`demi-runner` binary, exercised against a **bare AgentServer** in tests —
no product integration, no device registry yet. First because it depends
on nothing else and is the design's only greenfield contract — the
earlier it exists, the earlier its implementation problems surface.

**M2 — Backend skeleton + virtual default (first end-to-end node)**
`@demicodes/backend` serving the conversation stream + the Web API
multi-user-shaped (stub user); the storage module (schema, numbered `.sql`
migrations, thin SQL layer); conversation persistence + session index;
`@demicodes/host-virtual` with its local-dir blob backend as the default
target. No frontend work — all acceptance is test-level (in-process
`AgentClient`). Providers are **operator-assembled** exactly like the old
dev product (env keys / operator logins) — the vault is explicitly out of
scope here. Accept: a zero-setup virtual conversation with
portable-command tools; client detach mid-turn reattaches to the running
turn; cold history readable.

**M3 — Storage final shape**
The final storage layout at N=1: `control.sqlite` +
`conversations/<id>.sqlite`, the block-row journal in the agent
persistence contract (streaming appends block rows — no whole-transcript
rewrites), transcript media out of the transcript via `source.ref` into
the blob store, and `host_store` scoped into the conversation databases.
`ControlService` exists from here as the interface in front of
`control.sqlite` (Local implementation only; the RPC realization is
M10's).

**M4 — Runner productized**
Claim-by-token flow, device registry with online status, backend harness
host resolution to remote Hosts. Accept: a session executes real shell
commands and file edits on a claimed device; runner disconnect surfaces as
tool errors without losing the session; reconnect resumes.

**M5 — LLM module + credential vault + metering + Claude Code**
Two acceptance steps in order:
1. *BYOK + metering* (no dependencies): vault key storage (instance-secret
   encryption at rest), per-user provider assembly for the API-key providers
   keyed by `(connectionId, modelId)`, usage ledger + enforcement. This is
   the product's minimum viable form — a user pastes a key and chats — and
   stands alone as a usable point.
2. *Subscriptions + Claude Code* (depends on M4): provider device-login
   flows + refresh in the vault, the Anthropic passthrough, claude-code
   sessions spawning their CLI on the session's runner. Accept: turns with
   every provider against mock LLM endpoints, runner holding zero
   credentials; CLI chain end-to-end through the passthrough (skip when no
   `claude` binary); real-subscription smoke manual only, never an ungated
   test.

**M6 — Target switching + attachments (mechanisms + endpoints only; UI at M8)**
Turn-boundary switching + context injection + the out-of-virtual tmp-dump
(model relocates); workspaces CRUD; offline-target degradation (read/chat via
virtual);
message-attachment upload (inline media content blocks) and workspace file
drop (Host RPC `writeFile`). Accept: switch, upgrade, and offline flows each
covered by integration tests; an uploaded image round-trips through a
StubProvider turn and the checkpoint.

**M7 — Multi-user systems (API-level, no UI)**
Real auth (username/password, cookie sessions, master/admin/user roles, no
registration, no recovery); user-management and instance-settings endpoints;
**shared/isolated instance-mode enforcement** (admin-only connections in
shared mode); tenant-isolation authz matrix. Everything test-accepted
against the API — at the end of M7 the entire API surface is complete and
frozen.

**M8 — Web UI (its own dedicated milestone, last among feature work)**
The **entire `@demicodes/web` package** built in one concentrated phase per
the layout design — scaffold, workspace-grouped sidebar, chat view on
web-ui, settings dialogs (devices, connections, usage, user management,
instance settings), target picker — with the old dev product renamed
`web-demo` at this point. UI is deliberately last so that completed,
frozen functionality never forces UI rework; it consumes the M7-frozen API
and adds no new backend surface.

**M9 — Deployment packaging**
Docker images for runner and backend (backend image carries the built web
assets); a sample Litestream sidecar config (optional S3 durability at
N=1); end-to-end acceptance. Pure hosting work after interfaces are
frozen.

**M10 — Scaled deployment (post-v1)**
Everything the N>1 topology needs, gathered from the design sections:
`demi-controld` as a standalone process (`ControlService` RPC server on
Hono + `RemoteControlService` client, internal listener, service-token
auth), the workers' short-TTL auth token cache, S3 blob backend for
host-virtual, Litestream deployment config (every node, `dir` + `watch`),
routing-key plumbing (login cookie + runner route key) and the sample
reverse-proxy config with the static user→worker map, the user-migration
procedure (stop → restore → remap), multi-instance smoke. No application code path changes — the `ControlService` seam and
the storage shape are already final from M3; that is the point.

Independent branch, any time after M2: the virtual-target JS command
(WASM-sandboxed QuickJS; own design record first).

Deliberately deferred: fs RPC batching/caching (only when measurements
demand), per-wire usage reconciliation.

## Milestone verification

Three tiers, matching repo conventions: (1) model-free automated tests
(StubProvider + local mocks, scoped `bun test packages/<pkg>`, CI-gating);
(2) env-gated real-credential smoke (`real-*.e2e.test.ts`, `DEMI_*_E2E`,
manual pre-release only — real models are never a merge gate); (3) manual
checklists only for UI look-and-feel and packaging smoke.

| M | Verification |
|---|---|
| M0 | Spawn-injection + `buildClaudeEnv` overlay assertions (no CLI). Capability-flag tests. Host-switch integration: StubProvider session runs turn 1 against LocalHost A, turn 2 against LocalHost B; assert context block injected, per-Host BashEnvironment isolation, transcript continuity. |
| M1 | Protocol codec round-trips (portable JSON incl. `Uint8Array`). Remote-Host integration against a bare AgentServer: session executes `cat`/`tee`/spawn on a runner in a temp dir; kill the runner mid-command → tool error, session continues; reconnect → next command succeeds. |
| M2 | Backend integration in one test process: browser-side `AgentClient` (in-process transport) + virtual-Host session; detach client mid-turn with a slow StubProvider → turn completes → reattach sees the full result (covers refresh-immunity and binding-close-must-not-abort); cold-history read equals live transcript; portable commands work, spawn fails with the upgrade message. |
| M3 | Block-row persistence: a streamed turn appends rows (no whole-transcript rewrite observable); restore from `control.sqlite` + `conversations/<id>.sqlite` equals the live transcript; media blocks round-trip through `source.ref` + blob store; per-conversation `host_store` isolation. |
| M4 | Claim-flow integration (unclaimed → claim → reconnect with device token; bad/revoked token; claim-token expiry). Backend host routing to a claimed device's remote Host; device online status follows the socket. |
| M5 | Step 1: vault key storage + per-user assembly unit tests; ledger aggregation from StubProvider usage. Step 2: login-flow state machines against mock auth endpoints + refresh; passthrough mock upstream asserts token swap and single request class; claude-code-on-runner chain with the real CLI against a mock upstream, skipped when no `claude` binary. Tier 2: one gated real-subscription smoke per provider. |
| M6 | Switch integration, all directions unconstrained: real→real (files stay + honest context block; same-device note when applicable), virtual→real (files land in the target tmp dir, context block names the path, no code-side placement), real→virtual (fresh virtual fs + context block), mid-turn switch refused, concurrent switch has one winner; offline target → session readable and chattable on virtual. |
| M7 | Tenant-isolation authz matrix (every API action by user A against user B's data asserts denial); instance-mode enforcement (shared: non-admin connection writes rejected, everyone reads the instance connections; isolated: users see only their own); device revoke + re-claim via the API. |
| M8 | Tier 3 manual checklist over the full layout design, including a sweep of the "everything Demi implements gets exposed" list (steer, queue, abort, retry, resume, compact, `set_provider`, `shell_write`). |
| M9 | Tier 3 scripted smoke: build both images, claim a containerized runner against a local backend, run one full turn end-to-end; optional CI stage. |
| M10 | `ControlService` contract tests run against both implementations (Local in-process / Remote over a real controld); two local workers + one controld behind a mapped proxy — user pinned to one worker, migration (stop → restore conversation files from the replica → remap) preserves history; blob-store S3 round-trip against a local S3-compatible server; domain errors survive the RPC wire with `code` intact. |

Test modules and their coverage get documented per milestone as they land, per
the repo's design-record rules.
