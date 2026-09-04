# Demi Next: Storage

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Design (implemented through M6; home-image store in M11; S3 backends and controld in M15) |
| Scope | The two databases, `ControlService`, the blob store, the home-image store, replication, the N>1 topology |

## The split

**SQLite is the only dialect, in both topologies.** The storage split
follows the write-frequency line:

- **`control.sqlite`** — one per deployment, the control-plane data: users,
  auth, devices, conversation index, workspaces, attached hosts, providers/vault,
  ledger, attachment metadata. Low write rate (the hottest writer
  is one ledger row per provider request), read-heavy (auth check per
  request, absorbed by a short-TTL token cache).
- **`conversations/<id>.sqlite`** — one file per conversation, the data
  plane: the transcript as **one row per block** (the journal — streaming
  persists by appending block rows, never by rewriting a checkpoint JSON),
  plus session state, that conversation's `host_store` scope, and the
  **tree** of its hostless filesystem (paths and metadata; the bytes are
  in the blob store). High write rate, but each file has exactly one
  writer and the files never contend.
- **Blob store** — attachment bytes, transcript media (`source.ref`) and
  the contents of hostless files, content-addressed within each user's
  namespace at `blobs/<userId>/<sha256>`:
  local directory at N=1, S3 at N>1. Bytes never enter a database.
- **Home-image store** — managed hosts' home images, one **named, mutable,
  owner-bound** object per owner (`homes/<ownerId>.ext4`), overwritten in
  place on hibernate (temp + atomic rename), streamed in and out: local
  directory at N=1, S3 at N>1. Not the blob store: an image has one current
  version, not a history of content hashes (`managed-hosts.md`).
- **Litestream** watches the data directory (`dir` + glob + `watch`) and
  continuously replicates every `*.sqlite` to S3: asynchronous, loses at
  most about the last sync interval on node death; restore is snapshot +
  LTX replay, point-in-time capable. Optional at N=1, required at N>1.

**No ORM, no query builder**: a hand-rolled thin storage module with
hand-written SQL and numbered migrations, written for SQLite alone.

## Topology (N>1)

Workers are fully symmetric; the control plane is a dedicated internal
service:

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
 │ conversation    │          │                 │       │                 │
 │ hot path:       │          │     (same)      │       │     (same)      │
 │  WS stream,     │          │                 │       │                 │
 │  cold transcript│          │                 │       │                 │
 │   │ block append│          │                 │       │                 │
 │   ▼             │          │                 │       │                 │
 │ conversations/  │          │ conversations/  │       │ conversations/  │
 │  <id>.sqlite ×n │          │  <id>.sqlite ×n │       │  <id>.sqlite ×n │
 │ managed VMs     │          │ managed VMs     │       │ managed VMs     │
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
      blobs/<userId>/<sha256>   attachment bytes + transcript media + hostless files
      homes/<owner>    managed-host home images
```

**N=1 is the same picture with the LB and the extra workers deleted: one
process = worker + controld fused.** `ControlService` is the in-process
implementation, no RPC; the data directory is byte-identical. That
homogeneity is a design requirement: the storage shape never changes
between topologies, only the process placement does.

Interface topology — every endpoint by where its data lives:

```
 (a) control-plane endpoints — worker is a thin shell over one RPC call
   POST /api/auth/login ────── createSession ─────────▶ ┌─────────┐
   GET  /api/conversations ─── listConversations ─────▶ │ demi-   │──▶ control
   POST /api/devices/claim ─── claimDevice ───────────▶ │ controld│    .sqlite
   …(workspaces, attached hosts, providers, usage, admin) └─────────┘

 (b) auth check on EVERY authed request — RPC, blunted by a local cache
   any request ──▶ worker token cache (short TTL) ──miss──▶ resolveSession

 (c) conversation hot path — worker-LOCAL, never crosses the network
   WS /:id/stream ──▶ live session ──▶ conversations/<id>.sqlite (block append)
   GET /:id/transcript ─────────────▶ conversations/<id>.sqlite (cold read)
   hostless Host fs ops ────────────▶ conversations/<id>.sqlite (files tree) + blob store (bytes)
   managed VM lifecycle ────────────▶ homes/<owner> (worker-local process, store upload)

 (d) mixed endpoints — local work + independent control-plane appends
   WS stream, turn ends ─┬─▶ <id>.sqlite (blocks, local)
                         ├─▶ appendUsage ───────▶ controld (ledger row)
                         └─▶ touchConversation ─▶ controld (updated_at, title)
   POST /api/attachments ─┬─▶ blob store (bytes)
                          └─▶ putAttachmentMeta ▶ controld (metadata row)

 (e) runner WS — terminates on the worker; controld sees identity only
   claim ──▶ claimDevice (once) · hello ──▶ token cache / resolve
   fs / spawn / job / rpc streams ⇆ live sessions (worker-local only)
```

Invariants this topology enforces:

- The public API exists only on workers; `demi-controld` has no public
  endpoint. Workers never touch `control.sqlite`; controld never touches
  conversation files, blobs or home images.
- The RPC surface is the `ControlService` interface mapped 1:1 (Hono +
  `POST /rpc/<method>`, plain JSON, domain errors as `{code, message}`
  rebuilt by the client). One call = one atomic operation; transactions
  never span calls; SQL never crosses the wire.
- Every high-frequency write is worker-local; every controld call is
  low-rate or cache-absorbed.
- No cross-database transactions exist anywhere: the (d) pairs are
  independent appends with no invariant between them.
- User→worker assignment is partitioned, not balanced per request; a user
  is pinned to one worker; rebalancing = migrating users.

Naming: interface `ControlService`, implementations `LocalControlService`
(in-process SQL — the N=1 backend and controld itself) and
`RemoteControlService` (the workers' RPC client); process `demi-controld`;
database `control.sqlite`. "Control plane / data plane" are prose names
only; the `*Store` suffix stays reserved for storage backends.

## Schema

`control.sqlite` (final state, no speculative columns):

```
users                   id, username, password_hash(argon2id), role(master|admin|user), created_at
web_sessions            token_hash(sha256 of the cookie token), user_id, expires_at
conversations           id, user_id, title, archived, workspace_id(NULL), host_device_id(NULL),
                        pending_switch_json(NULL), provider_id, model_id, created_at, updated_at
                        ← workspace_id and host_device_id mutually exclusive; both NULL = hostless
                        ← pending_switch_json: the switch the next turn announces ({from, to}), then NULL
conversation_hosts      conversation_id, device_id, name, cwd, attached_at
                        ← the attached hosts; UNIQUE (conversation_id, name); cwd = where the last shell there ended
workspaces              id, user_id, device_id, path, name, created_at
devices                 id, user_id, kind(user|managed), name, platform, token_hash,
                        owner_conversation_id(NULL), owner_workspace_id(NULL),   ← managed only
                        claimed_at, last_seen_at
providers               id, owner_user_id(NULL in shared mode), provider_type, credential_kind, label,
                        config(encrypted: key, endpoint, protocol, vendor id,
                        typed model list — or the subscription marker), created_at
                        ← one subscription entry per (owner scope, provider_type), enforced by
                          a partial UNIQUE index with the shared NULL owner mapped to one scope
usage_ledger            id, user_id, conversation_id, provider_id, model_id,
                        input_tokens, output_tokens, cache_tokens…, created_at
attachments             id, user_id, media_type, size_bytes, sha256, created_at
```

`conversations/<id>.sqlite` (shape owned by the agent persistence
contract):

```
blocks           one row per transcript block, append-only during streaming
session state    checkpoint fields other than the transcript
host_store       scope, key, value_json  ← this conversation's scope
files            path, kind(file|dir), mode, mtime, size, sha256(NULL for dir)
                 ← the hostless filesystem's tree; bytes in the blob store by sha256;
                   emptied once the conversation has a home image
```

## The hostless filesystem and the home image

A conversation's files have two forms, one per phase, and one conversion
between them:

- **Before a machine** — the `files` tree plus blobs, served to tinybash
  and the root commands as `@demicodes/host-virtual`'s `Host`. Copying a
  file copies a row; the quota counts bytes referenced by the tree.
  Workspace files dropped into a hostless conversation land here.
  Concurrent appends to one path are serialized through the filesystem
  backend's per-path queue, including the blob read and write; independent
  paths proceed concurrently and a failed append does not block later ones.
- **The upgrade** — the backend materialises the tree into a directory
  (modes and mtimes included; the tree holds no symlinks) and runs `mke2fs -d <dir>` to
  produce the home image with its contents in one step: no mount, no
  root, no guest cooperation. The directory becomes `/demi` inside the
  image, which the guest mounts at `/home`; the files carry the backend
  user's ownership until the guest's first boot chowns them to the guest
  user (`demi.firstboot=1` on the kernel command line, that boot only).
  `/tmp` is materialised under the home's `.tmp` directory. The image
  goes to the home-image store and the VM boots with it; the `files` rows
  are then deleted. Blobs stay — they are content-addressed and may be
  referenced elsewhere.
- **After** — the home image is the only form (`managed-hosts.md`).
  There is no way back to hostless, so no reverse conversion exists.

A Cloud workspace's first image is the same `mke2fs` on an empty
directory.

Notes: pending claim tokens live in memory (an unclaimed runner socket
holds them; a restart reprints); claim tokens are 128-bit random,
single-use, expiring, rate-limited per user; online status is runtime
state, `last_seen_at` display-only. `providers.config` is encrypted at
rest with an instance secret (generated into the data directory on first
start; a shared secret across instances at N>1). Ledger granularity: one
raw row per provider request as `TokenUsage` events arrive; aggregation at
query time.

## Pluggability

- Conversation state is fully behind `HostStore` (four methods) —
  checkpoints and subagent records. The backend implements a DB-backed
  `HostStore` and composes it into every Host it hands the harness. The
  DB-backed store provides atomic `writeJson` and serves `list` and bulk
  reads efficiently.
- The hostless filesystem is behind the `Host` fs contract
  (`@demicodes/host-virtual` over the `files` tree and the blob store).
- The blob store is put/get by content hash within a user namespace, with
  two backends (directory, S3). `UserBlobStores` resolves uploads and HTTP
  downloads by authenticated user, and session persistence, transcript
  media and hostless files by conversation owner. `ConversationStores`
  and `AgentServer` receive a per-session BlobStore factory; child sessions
  inherit their root's store. A hash identifies bytes within that scope
  and grants no access to another user's namespace.
  The home-image store is streaming write/read by owner id with the
  same two backends. Both live in `@demicodes/backend`.

## Precedents

Per-tenant SQLite files with a single owning process (Bluesky PDS);
symmetric data nodes with a dedicated low-write metadata service (HDFS
NameNode, TiDB PD, Kafka controller, Kubernetes control plane); a service
exclusively owning its database behind a domain API; an HTTP service
fronting SQLite (Grafana, Gitea, Headscale); user-sharded SQLite control
planes with tenant migration (Tailscale); streaming SQLite replication to
S3 (Litestream). Alternatives weighed during the review are archived in
`progress.md`.
