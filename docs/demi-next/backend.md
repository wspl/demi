# Demi Next: The Backend

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Design (implemented through M6) |
| Scope | The `@demicodes/backend` program: modules, deployment topology, routing, the Web API |

## One program, one architecture

Scaling is running more copies of the same program plus one internal
control-plane process (serverless hosting stays out of scope so its
constraints cannot leak into the interfaces):

```
Self-host (one instance):

  browser ─────────┐
                   ├──►  demi-backend  ──►  control.sqlite
  runner ──────────┘     (complete: Web API │  + conversations/<id>.sqlite (per conv)
                          + sessions + vault│  + blob dir + homes dir
                          + runner mgmt)    └─ (litestream ──► S3, optional)


Scaled (same program × N + one internal control-plane service):

                        ┌──►  demi-backend #1 (users a,b) ─► own conversations/*.sqlite ─┐
  browsers ──►  router  ├──►  demi-backend #2 (users c,d) ─► own conversations/*.sqlite ─┼─► demi-controld
  runners  ──►  (pins   ├──►  demi-backend #3 (users e,f) ─► own conversations/*.sqlite ─┘   (internal RPC,
                by user) └──►  …                                every node: litestream ─► S3   control.sqlite)
```

Every instance is a **complete** backend for its assigned users — user x's
HTTP, conversation sockets, runner sockets, hostless Hosts, managed VMs and
CLI processes are all pinned to the instance the user→worker map names. The
affinity is natural because conversations, devices and (isolated mode)
connections are all user-owned, so nothing stateful ever crosses instances.
Self-host is the N=1 degenerate form with no router. v1 milestones implement
N=1 only.

Routing — the router is an off-the-shelf reverse proxy selecting the
upstream from a routing key; we develop no routing code and ship a sample
config:

- **The routing key**: at login the backend sets a uid cookie (browser
  traffic); at claim time it hands the runner its owner's route key
  alongside the device token, and the runner sends it as a header on every
  reconnect (device traffic). Same key ⇒ same map entry ⇒ one instance.
  Requests without a key yet (login) may land on any worker; those
  endpoints only talk to the control plane.
- **Routing happens at connection establishment only.** Established
  WebSockets are never rerouted; during a scale event in-flight turns
  finish on the old instance.
- **Scale events move users explicitly**, and a moved user's experience is
  a backend restart: stop the user's sessions on the source, `litestream
  restore` their conversation files on the target, update the map; runners
  reconnect to the new home. Managed VMs are hibernated on the source and
  woken on the target from the home-image store.
- **Control-plane access is the only cross-instance traffic** — the
  `ControlService` RPC (`storage.md`).

## Modules

Spoken of as modules, not separate services:

- **Conversation module**: AgentServer/AgentSession hosting, transcript
  persistence, session index, cold-history reads on the same rendering
  path as live (a full-sync `transcript_reset`), compaction, session
  concurrency via client-owned session ids and the ownership registry.
- **Command module**: assembles the roots (`demi` from
  `@demicodes/coding-agent` plus the backend-contributed `host` group),
  builds and serves the manifest, runs hostless conversations' tool calls
  in tinybash over an in-process loader, executes `rpc` commands arriving
  from runners (`commands.md`).
- **LLM module**, **credential vault**, **usage accounting**:
  `providers-and-vault.md`.
- **Runner management module**: device registry (claim tokens, device
  tokens, online status = socket state, one live connection per token), the
  runner-protocol server, per-conversation `RemoteHost` handles
  (`@demicodes/host-remote`) over connected runners, the rpc relay,
  brokered cross-host transfers (`runner.md`).
- **Managed hosts module**: the `ManagedHostProvisioner` (Firecracker under
  jailer via the privileged helper), images, the home-image store,
  lifecycle (`managed-hosts.md`).
- **Auth module**: users and roles, password hashing (argon2id), the
  cookie sessions with their sliding expiry, the login lockout. Device
  claiming is the runner module's, over the session user.

## Media by reference

Nothing the backend sends to a browser inlines bulk bytes. The
conversation-scoped transport rewrites every outbound frame that carries
transcript blocks (`transcript_reset`, `transcript_patch`, and the
subagent pair): each inline media source becomes `{ type: 'ref', ref,
mediaType }` — `ref` the blob's content hash, the same form the block rows
hold at rest — and the page fetches `GET /api/blobs/:sha256?type=<media
type>` (cookie-authenticated, `immutable`, cached for a year). Providers
still receive inline bytes: the conversation module resolves references
before handing a message to the provider runtime. The same rule governs
attachments in both directions (`product.md`): an upload's `ref` in a
`send` frame is the attachment id, resolved inbound.

## Web API (browser ↔ backend)

Two kinds of traffic:

1. **Application data — plain HTTP REST.** Cookie auth, standard status
   codes, cacheable reads, upload progress for free. Errors: HTTP status +
   `{code, message}`, codes as stable strings. No `/v1` prefix — frontend
   and backend ship together.
2. **The live conversation stream — one WebSocket per open conversation**,
   `WS /api/conversations/:id/stream`, carrying Demi's agent frame protocol
   (`ClientFrame`/`ServerFrame`). The execution target and cwd are resolved
   server-side from the conversation record; the browser never names a cwd.

`@demicodes/web-ui` consumes a transport-agnostic client interface, backed
with fetch by the product shell. No server push beyond the stream; pages
poll on open and on an interval.

**Authentication.** One session gate covers `/api/*`: the `demi_session`
cookie (`HttpOnly; SameSite=Lax; Path=/`, `Secure` over https) names a
row in `web_sessions` by the SHA-256 of its 256-bit token; no row or an
expired one is 401 `unauthenticated` and the cookie is cleared. The
session lives 30 days from its last renewal; a request arriving with
under 15 days left renews it (a fresh `Set-Cookie`), so an active
browser never signs out and a silent one does. The stream WebSocket and
the blob route ride the same cookie. Exempt from the gate: `/api/setup`
and `/api/auth/login` (the entrances) and `/api/runner`, `/api/transfers`
(device-token authenticated; runners never hold a cookie). Answers:
another user's object 404 as if absent, a role short of the action 403,
no session 401. Login failures lock the username for a minute after
five in a row.

**The connection scope.** The instance mode names whose connections a
caller works with: in shared mode the instance's (owner null; created,
tested, logged in and deleted by admins, listed and used by everyone),
in isolated mode the caller's own. Listing, the catalog, the model
selection on a conversation and the provider a session resolves all go
through the same scope, so a connection outside it is unknown — 404 on
the API, no provider at the session. The mode is fixed once providers
are configured: a start under the other mode with connections in the
table refuses with the reason.

| Resource | Endpoints | Lands in |
|---|---|---|
| setup | `GET /api/setup` (`{ needed }`), `POST /api/setup` (the master account, once; signs it in) | M12 |
| auth | `POST /api/auth/login`, `POST /api/auth/logout`, `GET /api/auth/me`, `PUT /api/auth/password` (the caller's own) | M12 |
| conversations | `GET/POST /api/conversations`; `PATCH /api/conversations/:id` (rename/archive/unarchive/target/model); `GET /api/conversations/:id/transcript`; `WS /api/conversations/:id/stream` | M2 |
| models | `GET /api/models` (the caller's connection scope's catalog, grouped by connection) | M2; scoped M12 |
| devices | `GET /api/devices`, `POST /api/devices/claim`, `DELETE /api/devices/:id`, `GET /api/devices/:id/fs?path=…`, `POST /api/devices/:id/fs` (create directory) | M4 |
| workspaces | `GET/POST /api/workspaces`, `PATCH/DELETE /api/workspaces/:id` (never touches files); creation takes `cloud: true` in place of a deviceId | M6; cloud flag M11 |
| connections | `GET/POST /api/connections`, `DELETE /api/connections/:id`, `POST /api/connections/:id/test`, `POST /api/connections/subscription-login` + `GET …/subscription-login/:id` — in the caller's connection scope: the instance's in shared mode (writes are admin-only), the caller's own in isolated mode | M5; scoped M12 |
| usage | `GET /api/usage` (the caller's), `GET /api/usage/instance` (shared mode, admins: by user) | M5; instance view M12 |
| attachments, blobs | `POST /api/attachments` (returns a reference id), `POST /api/conversations/:id/workspace-files`, `GET /api/blobs/:sha256` | M6; blobs M9 |
| transfers | `PUT /api/transfers/:id` (source runner), `GET /api/transfers/:id` (destination runner); device-token authenticated, single-use, piped in flight (`runner.md` § Transfers) | M9 |
| grants | `GET/POST/DELETE /api/conversations/:id/grants` | M11 |
| commands | `GET /api/commands/manifest`, `GET /api/commands/modules/:hash` (the manifest source for a standalone command-mode shell) | M8 |
| admin | `GET/POST/PATCH /api/users`, `GET /api/settings` (the instance mode, read-only: it is startup configuration) | M12 |

## Packages

`@demicodes/backend` is a product leaf: nothing imports it. Its production
dependencies are the agent, coding-agent, core, provider, the provider
runtimes, shell (the Host contract and command types; `hostless` for the
hostless shell environment; `node` for the data directory's filesystem),
host-virtual and host-remote (the two Hosts it injects into the agent),
command-loader, runner-protocol and utils, plus `hono` on Bun. The module
directories mirror the modules above (`docs/package-boundaries.md`).
