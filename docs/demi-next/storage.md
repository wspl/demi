# Demi Next: Storage

The backend separates shared product records from each conversation's frequently
updated state. SQLite is the database dialect in both the single-backend design
and the intended multi-worker deployment. Attachment bytes and managed-machine
disks use separate file stores because their ownership and retention differ.

## Ownership and layout

```text
Backend data directory
|
+-- control.sqlite                deployment-wide product records
+-- conversations/<id>.sqlite     one agent tree per conversation
+-- blobs/<userId>/<sha256>        user-owned attachment and media bytes
+-- changes/<conversationId>/     a conversation's edited-file contents per command
+-- machines/<deviceId>/          managed-machine disk generations
+-- vault/<providerId>/           subscription credential pools
+-- instance secret              encrypts provider configuration
```

The names identify storage responsibilities; deployment options supply the actual
roots. `backend/storage` owns SQLite and attachment stores.
`backend/managed` owns the machine-image store. `backend/vault` owns credential
storage and access. Working project files belong to execution devices and are
accessed through `Host.fs`; they are not conversation database content.

| Store | Owns | Writer |
|---|---|---|
| `control.sqlite` | Accounts, auth sessions, preferences, devices, workspaces, conversation index, providers, model catalogs, usage, attachment metadata, operation records | `LocalControlService` |
| Conversation database | Root and subagent nodes, checkpoint state, transcript blocks, command history, conversation Host storage | The backend owning that conversation |
| User blob namespace | Uploaded bytes and transcript media addressed by content hash | Backend upload and media persistence paths |
| Change store | Both sides of every file a command edited, bound to the conversation ([Edit tracking](edit-tracking.md#the-change-store)) | Backend command completion |
| Machine-image store | Published system/home disk generations and their manifests | Managed-host lifecycle |
| Credential pool | Subscription account secrets and active-account selection | Provider credential implementation invoked by the vault |

Handwritten SQL sits behind a thin database interface. Numbered schema changes
run in order, each in a transaction. SQLite uses WAL and foreign keys. Product
modules call `ControlService` domain methods rather than issuing control SQL.
The schema source is `backend/storage/migrations.ts`; this document defines data
meaning and atomicity rather than duplicating every SQL column.

## Control records

`ControlService` owns these groups:

- **Identity:** `users` stores case-insensitively unique email, nickname, password
  hash, role, and creation time. `web_sessions` stores a hash of the session token,
  user identity, and expiry. `email_challenges` stores one pending address-change
  challenge per user, including code hash, password hash at issue, expiry, send
  time, and failed-attempt count. Confirming the challenge updates the email and
  consumes the challenge in one transaction. Account behavior belongs to
  [Product](product.md) and its HTTP contract to [Backend](backend.md).
- **Preferences:** `user_preferences` stores validated appearance and shortcut
  overrides and the last explicit model selection for new conversations. A patch
  merges specified fields in one transaction so independent
  edits do not overwrite each other.
- **Devices and workspaces:** `devices` stores ownership, kind, identity, token
  hash, and timestamps. A partial unique index permits one managed device per
  user. `workspaces` names directories on devices; deleting a workspace never
  deletes its files. Online status comes from live connections, not `last_seen_at`.
- **Conversation index:** `conversations` stores ownership, title, archive and
  pin state, ordering, read revision, target selection, target context revision,
  last switch, Cloud reset marker, and provider/model identifiers. The full agent
  checkpoint belongs to the conversation database. `conversation_hosts` stores
  attached devices with a name unique within the conversation and their last cwd.
- **Operations:** `conversation_fork_operations` reserves a destination ID and
  records source boundary, owner, target, full model selection, title, creation
  time, and attached hosts. `managed_operations` records reset intent and progress
  under device and operation IDs. These records make interrupted multi-step work
  discoverable; they are not cross-database transactions.
- **Providers:** `providers` stores owner scope, family, credential kind, label,
  and encrypted configuration. A partial unique index enforces one subscription
  entry per owner scope and family, including the shared scope.
  `model_catalogs` stores one validated cache record per provider entry, removed
  with that entry. See [Providers](providers-and-vault.md) and
  [Model catalog caching](../model-catalog-cache.md).
- **Usage and attachments:** `usage_ledger` stores user, conversation, provider,
  model, token counts, and observation time. `attachments` stores owner, media
  type, byte length, content hash, and creation time. Neither table contains the
  attachment bytes. Accounting limitations are defined in
  [Providers](providers-and-vault.md#implementation-limits).

Conversation and workspace `sort_order` represent explicit user ordering;
activity timestamps do not reorder them. A read acknowledgement advances
`read_revision` with `MAX`, so an old browser cannot move it backward. The
conversation module refuses a revision beyond current output.

Projects retain their explicit order; conversations are ordered within their
project and pin partition. New projects append and new conversations enter at the
front. Rename, archive/restore, and target changes retain sort positions, with ID
as the stable tie-breaker. Reorder writes one partition atomically and rejects
archived rows or cross-partition targets. [Web API](web-api.md) owns the request
contract.

## Conversation state and transactions

Each `conversations/<id>.sqlite` contains one agent tree. Its root node ID is the
conversation ID; child nodes are subagents. The backend supplies this database
through the agent's `AgentTreeStore` contract, independently of any execution
Host. The agent persistence contract is defined in [Subagents](../subagent.md).

| Table | Meaning |
|---|---|
| `nodes` | Parent relationship, description/profile, spawn metadata, close result or failure, completion-delivery state, checkpoint state, block count, command and output revisions |
| `blocks` | One serialized transcript block per node and block index |
| `command_snapshots` | Immutable complete command-state maps indexed by node and revision |
| `session_boundaries` | History cutoffs linked to a command revision |
| `host_store` | Key/value state in the conversation's Host storage scope |

For example, saving a streamed assistant block updates that block's row and the
node checkpoint together. It does not serialize the whole transcript into
`state_json`. Rewinding history deletes rows beyond the new block count and
restores the corresponding command state. The journal is therefore a sequence
of indexed blocks, not an append-only database.

Creating a node inserts its identity and initial checkpoint atomically. Saving
updates changed block rows, deletes the truncated tail, writes checkpoint and
command state, and marks child completions carried by that checkpoint as
consumed, all in one transaction. Closing records phase, time, result or failure,
and pending completion delivery atomically. Deleting a node cascades to its
descendants and their dependent rows.

Changed output advances `output_revision` in the checkpoint transaction;
user input alone does not. `ConversationStores.summary` reads phase, output
revision, and the latest terminal block without loading the entire transcript.
The conversation module combines persisted facts with live activity to produce
browser summaries.

`ConversationStores` returns stable database handles that open their underlying
SQLite connection on demand. It retains at most 64 open connections by default
and closes the least recently used connection when the limit is exceeded. A
later operation reopens it; closing an idle connection does not discard the
conversation. Backend shutdown closes the retained connections.

## Attachment and transcript media

`UserBlobStores` selects a blob namespace by authenticated user for uploads and
downloads, and by conversation owner for transcript persistence. A content hash
identifies bytes only within that namespace. Knowing another user's hash grants
no access to their blob.

Before saving transcript blocks, the tree store externalizes inline media from
both root and subagent blocks. Blocks then contain media references. Loading a
session for inference rehydrates the bytes; cold browser history can keep the
references. The agent does not manage a product blob store. Browser delivery and
uploaded attachment resolution are defined in
[Backend media handling](backend.md#media-by-reference).

The directory store hashes bytes with SHA-256, writes a temporary file, and
renames it to the hash path. Repeated writes of the same content reuse that
path. A failed lookup returns absent only for an invalid hash or missing file;
other read errors propagate. The S3 implementation must preserve the same
user namespace and put/get behavior.

Blob publication precedes the database checkpoint that references it. A database
failure can leave an unreferenced blob; it must not leave a committed block
pointing at an incompletely published upload. There is no transaction spanning
SQLite and the blob store. Blob collection and account deletion require an
explicit retention policy before automated reclamation is introduced.

## Machine disk generations

A managed machine has working system/home images and a committed generation:

```text
machines/<deviceId>/
+-- current.json
+-- generations/<generationId>/
    +-- manifest.json
    +-- system.ext4
    +-- home.ext4
```

A manifest identifies the device, pinned base version, and both writable images.
Both images must be durable before publishing the current-generation pointer.
Readers use one manifest and never combine unrelated checkpoints. A reset may
reference the already durable home image of its source generation; referenced
images cannot be collected.

Capturing a running machine pauses it while copying its writable volumes. The
snapshot preserves filesystem state, not an application's multi-file transaction.
A failed publication preserves the working images and previous committed
pointer so saving can be retried. The directory store retains the current
complete generation and its predecessor and removes older unreferenced generation
directories after successful publication. Conversation or workspace deletion
never collects machine disks.

[Managed hosts](managed-hosts.md) owns boot, checkpoint, reset, and recovery order.
An S3 image adapter must preserve publication atomicity with conditional manifest
updates; uploading two image objects independently is not a committed generation.

## Multi-worker storage placement

The target deployment adds a single internal control service while retaining the
same SQLite schemas and ownership boundaries:

```text
Worker A                         Worker B
+-------------------------+      +-------------------------+
| Owned conversations DBs |      | Owned conversations DBs |
| Owned managed machines  |      | Owned managed machines  |
| RemoteControlService ---+--+ +--+-- RemoteControlService  |
+-------------------------+  | |  +-------------------------+
                             v v
                   +-----------------------+
                   | demi-controld         |
                   | LocalControlService   |
                   | control.sqlite        |
                   +-----------------------+

Workers: blob/image objects -> S3
Each SQLite owner: database replication -> S3
```

Workers serve the public API and own their assigned users' live state. Only
`demi-controld` opens `control.sqlite`; it never opens conversation databases or
machine disks. A single-backend deployment calls `LocalControlService` in-process
instead. [Backend deployment](backend.md) owns routing and user movement.

The remote control interface maps `ControlService` methods to private,
service-token-authenticated `POST /rpc/<method>` JSON requests. Domain errors
carry `{ code, message }`. Each method is one operation with any required local
transaction inside the service. SQL and transaction handles never cross the
wire. Authentication lookups can use a short-lived worker cache; conversation
block writes remain worker-local.

Independent usage and conversation-index updates do not commit atomically with
a transcript. Operations that require recoverable publication, such as a fork,
must use their recorded intent and completion checks. There is no general
transaction spanning the control database, conversation databases, and files.

The selected replication approach is asynchronous SQLite replication to S3 using
Litestream. It is optional for a local deployment and required for the target
multi-worker recovery design. Recovery restores a snapshot and subsequent
replicated log data after fencing the old writer. The recovery point is the last
successfully replicated state; an unhealthy replicator can lose more than one
configured sync interval. Machine images and blobs need their own durability and
restoration paths. All workers needing provider configuration share the instance
secret through deployment configuration.

## Implementation status and open decisions

The shipped backend constructs `LocalControlService`, local conversation
SQLite databases, `DirBlobStore`, and the directory machine-image store. It does
not implement `RemoteControlService`, `demi-controld`, S3 store adapters, routing
ownership/fencing, or Litestream provisioning and restore orchestration. The
multi-worker section defines required behavior, not an available deployment mode.

Subscription pool files are not encrypted by the database configuration cipher.
The directory blob store uses rename publication but does not explicitly fsync
file and directory data; its power-loss durability must be defined before
claiming the same durability guarantee as committed machine generations.

Before implementing multi-worker recovery, define ownership fencing, replicated
checkpoint readiness during user movement, credential-pool distribution and
recovery, and retention for deleted accounts and unreferenced blobs. These are
necessary parts of that deployment's correctness; the local file layout alone
does not resolve them.
