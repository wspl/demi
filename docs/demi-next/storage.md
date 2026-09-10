# Demi Next: Storage

| | |
|---|---|
| Date | 2026-09-08 |
| Status | Target architecture contract; acceptance tracked in `progress.md` |
| Scope | The two databases, `ControlService`, the blob store, the machine-image store, replication, the N>1 topology |

## The split

**SQLite is the only dialect, in both topologies.** The storage split
follows the write-frequency line:

- **`control.sqlite`** — one per deployment, the control-plane data: users,
  auth, devices, conversation index, workspaces, attached hosts, providers/vault,
  ledger, attachment metadata. Low write rate (the hottest writer
  is one ledger row per provider request), read-heavy (auth check per
  request, absorbed by a short-TTL token cache).
- **`conversations/<id>.sqlite`** — one file per conversation, the data
  plane: the session tree — one **node** row per agent (the root and every
  subagent under it: parent link, profile, spawn metadata, close and
  completion delivery) with each node's state row, and each node's
  transcript as **one row per block** (the journal — streaming persists by
  appending block rows, never by rewriting a checkpoint JSON) — that
  conversation's command snapshots, message cutoffs and `host_store` scope. High
  write rate, but each file has exactly one writer and the files never
  contend. The process keeps an LRU of open
  handles (64): a cold history read holds one only until other
  conversations are touched, and a conversation in use is always the most
  recent; the objects handed out for a conversation are stable and reopen
  the handle on demand.
- **Blob store** — attachment bytes, transcript media (`source.ref`), content-addressed within each user's
  namespace at `blobs/<userId>/<sha256>`:
  local directory at N=1, S3 at N>1. Bytes never enter a database.
- **Machine-image store** — one committed disk generation per managed device,
  owned by its user. Each generation manifest references a pinned base version,
  a writable system image and a home image. Both writable images become durable
  before atomic publication of the manifest. Local directory at N=1; an S3
  adapter with conditional manifest publication at N>1. This is separate from
  content-addressed attachment blobs (`managed-hosts.md`).
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
      blobs/<userId>/<sha256>   attachment bytes + transcript media
      machines/<deviceId>/    managed disk generations
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
   managed VM lifecycle ────────────▶ machines/<deviceId>/ (worker-local process, generation publication)

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
  conversation files, blobs or machine images.
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

`control.sqlite` (target schema):

```
users                   id, email(unique, case-insensitive), nickname, password_hash(argon2id), role(master|admin|user), created_at
web_sessions            token_hash(sha256 of the cookie token), user_id, expires_at
conversations           id, user_id, title, archived, target_json,
                        context_version, last_switch_json(NULL), cloud_reset_id(NULL), provider_id, model_id, created_at, updated_at
                        ← target_json: validated union cloud(path?) | device(deviceId, path) | workspace(workspaceId)
                        ← cloud resolves the user's unique managed device on demand
                        ← an explicit cloud path keeps a Fork in the source directory
                        ← each node persists its own observed context revision
conversation_forks      id, user_id, source_id, block_id, metadata_json
                        ← reserves one destination UUID per creation attempt
                        ← validated metadata: title, target, full model selection, creation time, attached hosts
                        ← published when the destination root has committed and its conversations row exists
conversation_hosts      conversation_id, device_id, name, cwd, attached_at
                        ← the attached hosts; UNIQUE (conversation_id, name); cwd = where the last shell there ended
workspaces              id, user_id, device_id, path, name, created_at
devices                 id, user_id, kind(user|managed), name, platform, token_hash,
                        claimed_at, last_seen_at
                        ← partial UNIQUE(user_id) WHERE kind = 'managed'
managed_operations      device_id, operation_id, operation_json, updated_at
                        ← validated reset record: id, baseVersion, phase, error(NULL)
                        ← operation_id unique per device; completed ids retained for retry
                        ← lifecycle admission permits one pending reset per device
                        ← allocation is the unique managed-device row; disk generation is in the image manifest
providers               id, owner_user_id(NULL in shared mode), provider_type, credential_kind, label,
                        config(encrypted: key, endpoint, protocol, vendor id,
                        typed model list — or the subscription marker), created_at
                        ← one subscription entry per (owner scope, provider_type), enforced by
                          a partial UNIQUE index with the shared NULL owner mapped to one scope
usage_ledger            id, user_id, conversation_id, provider_id, model_id,
                        input_tokens, output_tokens, cache_tokens…, created_at
attachments             id, user_id, media_type, size_bytes, sha256, created_at
```

`conversations/<id>.sqlite` (shape owned by the agent's `AgentTreeStore`
contract, `subagent.md` § Persistence; the root node's id is the
conversation id):

```
nodes            id, parent_id(NULL for the root), description, profile_name(NULL), metadata_json(NULL),
                 spawned_at, can_spawn, closed_phase(NULL while live), closed_at, result, delivered,
                 state_json, block_count, command_revision
                 ← state_json: the checkpoint fields other than the transcript (phase, queue, cwd, model, harness)
                 ← delivered: the parent has taken the completion (its checkpoint carries the wakeup,
                   the spawn command returned it, or the product took the closed frame)
blocks           node_id, idx, block_json  ← one row per transcript block, append-only during streaming
command_snapshots node_id, revision, values_json ← immutable complete command-state maps
session_boundaries node_id, block_id, edge, command_revision ← history cutoffs
host_store       scope, key, value_json  ← this conversation's scope
```

Create, save and close are each one transaction (`subagent.md` §
Persistence): a node row is inserted with its initial state and its first
message queued; a save writes the changed block rows, the state row, and
marks delivered every child completion the state row now carries; a close
writes the phase, time and result. Deleting a node deletes its descendants
and their rows.

## Machine disk generations

Working files live on devices, never in conversation databases or attachment
blobs. The backend writes a workspace file drop through Host RPC after obtaining
the selected device; message attachments remain backend blobs.

```text
machines/<deviceId>/current.json                 committed generation reference
machines/<deviceId>/generations/<generationId>/manifest.json
machines/<deviceId>/generations/<generationId>/system.ext4
machines/<deviceId>/generations/<generationId>/home.ext4
```

The manifest records device identity, base-image version and both image
references. A reset generation can reference the already durable home image
from its source generation. Referenced images cannot be reclaimed. Capturing a
running machine pauses it while copying both writable volumes. Publishing the
manifest is the commit point; readers never combine images from unrelated
checkpoints. This guarantees a filesystem-consistent snapshot, not application
transaction atomicity. `managed-hosts.md` owns boot, checkpoint and reset order.

Working images live separately while the VM runs or a save needs retry. Failed
publication preserves these files and the prior committed manifest. Recovery
fences old writers and resolves recorded operations before admitting work.
Account deletion and unreferenced-generation collection require an explicit
retention policy; project and conversation deletion never collect machine disks.

Notes: pending claim tokens live in memory (an unclaimed runner socket
holds them; a restart reprints); claim tokens are 128-bit random,
single-use, expiring, rate-limited per user; online status is runtime
state, `last_seen_at` display-only. `providers.config` is encrypted at
rest with an instance secret (generated into the data directory on first
start; a shared secret across instances at N>1). Ledger granularity: one
raw row per provider request as `TokenUsage` events arrive; aggregation at
query time.

## Pluggability

- Conversation state is behind the agent's `AgentTreeStore` contract: the
  backend's realization over the conversation database, one per
  conversation, handed to `AgentServer` by root session id. It knows no
  Host. Command state uses the node's immutable `command_snapshots` and
  `session_boundaries` under that contract. The node supplies a history-bound
  `CommandStorage` handle to every dispatched shell job. The backend also
  supplies a DB-backed `HostStore` as an independent Host storage facet.
- Device files are accessed through `Host.fs`, implemented by `RemoteHost`
  over the runner protocol. Backend-local storage uses its own filesystem adapter.
- The blob store is put/get by content hash within a user namespace, with
  two backends (directory, S3). `UserBlobStores` resolves uploads and HTTP
  downloads by authenticated user, and session persistence, transcript
  media by conversation owner. `ConversationStores`
  receives the per-conversation BlobStore factory and its tree store
  externalizes every node's media — root and subagents alike — into that
  namespace; the agent never sees a blob store. A hash identifies bytes
  within that scope and grants no access to another user's namespace.
  The machine-image store streams images by device and generation and atomically
  publishes their manifest, with the same two backends. Both live in `@demicodes/backend`.

## Precedents

Per-tenant SQLite files with a single owning process (Bluesky PDS);
symmetric data nodes with a dedicated low-write metadata service (HDFS
NameNode, TiDB PD, Kafka controller, Kubernetes control plane); a service
exclusively owning its database behind a domain API; an HTTP service
fronting SQLite (Grafana, Gitea, Headscale); user-sharded SQLite control
planes with tenant migration (Tailscale); streaming SQLite replication to
S3 (Litestream).

The directory machine-image store retains the current complete generation and
its immediate predecessor. After publishing and syncing the current pointer, it
removes older generation directories. Failed publication preserves the current
pointer and the source working disks; the next successful publication also
reclaims incomplete generation directories.

Account email changes use `email_challenges`: one pending row per user with the
new email, challenge ID, keyed code hash, password hash at issue, expiry, send
time and failed attempt count. Confirmation updates the email and consumes the
challenge in one control-database transaction. No prior nickname or email history
is stored. The initial schema defines the current design; there is no legacy
username migration.


## Browser preferences and conversation summaries

Control storage owns `user_preferences`, merged by explicit appearance/shortcut
fields, and conversation `pinned`, `sort_order`, `read_revision`; workspaces have
`sort_order`. Conversation node rows own `output_revision`, advanced in the same
checkpoint transaction that persists changed output blocks. Read acknowledgements
use MAX with the stored revision, so an older page cannot move read state backward.
`ConversationStores.summary` reads the root phase, output revision and latest
response/error/abort block without loading the complete transcript. The
conversation module combines those facts with live agent activity for the HTTP
summary. These fields are part of the initial schema; no old-data migration or
normalization path is provided.
