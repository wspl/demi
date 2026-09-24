# Storage

The backend separates shared product records from each conversation's
frequently updated state. SQLite is the database in both the single-backend
deployment and the multi-worker deployment. Attachment bytes and the contents
of edited files live in an object store, because their ownership and retention
differ from those of records. Cloud machine disks belong to the machine
manager, not to the backend.

## Ownership and layout

```text
Backend data directory (DEMI_BACKEND_DATA)
|
+-- control.sqlite                 deployment-wide product records
+-- conversations/<id>.sqlite      one agent tree per conversation
+-- blobs/<userId>/<sha256>        user-owned attachment and media bytes (object store)
+-- changes/<conversationId>/      edited-file contents per command (object store)
+-- instance-secret                seals credentials, unless configured
```

The names identify storage responsibilities; deployment options supply the
actual roots, and the two object-store namespaces move to an S3 bucket when
one is configured ([The object store](#the-object-store)). The `storage`
module owns the databases and the object store. The `vault` module owns
credential records and access; credentials are control records, never files.
The machine manager keeps each Cloud's disk generations in its own data
directory ([Managed hosts](../cloud/managed-hosts.md#provisioning)); the
backend stores only the Cloud's device record and its reset intent. Working
project files belong to execution devices and are reached through the
conversation's host access; they are not conversation database content.

| Store | Owns | Writer |
|---|---|---|
| `control.sqlite` | Accounts, auth sessions, preferences, devices, workspaces, exposes, conversation index, providers, model catalogs, usage, attachment metadata, operation records | The control service, on its database thread |
| Conversation database | Root and subagent nodes, checkpoint state, transcript blocks, command history | The shard of the user who owns the conversation |
| User blob namespace | Uploaded bytes and transcript media addressed by content hash | Upload and media persistence |
| Change store | Both sides of every file a command edited, bound to the conversation ([Edit tracking](../execution/edit-tracking.md#the-change-store)) | Command completion |

Both kinds of database are SQLite, reached through rusqlite with SQLite
compiled into the executable, so a deployment needs no system SQLite. A
transaction is a synchronous closure on its connection's own thread, so
nothing asynchronous happens inside one. Databases use WAL, foreign keys and a
five-second busy timeout, and stay plain SQLite files that Litestream can
replicate ([Multi-worker storage placement](#multi-worker-storage-placement)).
Each database's schema is one versioned step that `rusqlite_migration` applies
to a new database in a transaction, recording the version in SQLite's own
`user_version` field. Product modules call the control service's operations
rather than issuing control SQL. This document defines data meaning and
atomicity; the SQL lives in the storage module's schema.

## Control records

The control service owns these groups. It runs on one database thread: each
operation is one closure that runs in one transaction there, so no caller
holds a transaction open across a wait. Operations take owned, serializable
input, which the multi-worker control service also relies on
([Multi-worker storage placement](#multi-worker-storage-placement)).

- **Identity:** `users` stores case-insensitively unique email, nickname,
  password hash, role, and creation time. A partial unique index admits one
  master, so two concurrent setups cannot both create one. `web_sessions`
  stores a hash of the session token, user identity, and expiry; a login
  deletes expired sessions. `email_challenges` stores one pending
  address-change challenge per user, including code hash, password hash at
  issue, expiry, send time, and failed-attempt count. Confirming the challenge
  updates the email and consumes the challenge in one transaction. Account
  behavior belongs to [Product](../product/product.md#user-system), its HTTP
  contract to [Web API](../product/web-api.md#account-api), and sessions and
  lockout to [Backend](backend.md#authentication-and-ownership).
- **Preferences:** `user_preferences` stores validated appearance and shortcut
  overrides, the last explicit model selection for new conversations, and the
  locale the browser last reported. A patch merges specified fields in one
  transaction so independent edits do not overwrite each other.
- **Devices and workspaces:** `devices` stores ownership, kind, name and
  platform, the hash of the device's current token, and claim and last-seen
  times. The token hash is unique, so a runner's token finds its device
  through one index lookup; it is absent until the backend issues a token. A
  partial unique index permits one managed device per user. `workspaces` names
  directories on devices; deleting a workspace never deletes its files. Online
  status comes from live connections, not the last-seen time. `exposes` stores
  each [Host expose](../execution/expose.md#the-expose-record): id, owner,
  device, target address, creation and expiry time. Expiry, removal, a Cloud
  stop and device revocation delete rows; nothing updates a row except
  renewal.
- **Conversation index:** `conversations` stores ownership, title and its
  [origin](../product/product.md#conversation-titles), archive and pin state,
  ordering, read revision, target selection, target context revision, last
  switch, Cloud reset marker, provider and model identifiers, and the counts of
  messages the user sent and the last generated title had seen. The target is
  typed columns: its kind (Cloud, a device directory, or a workspace) and the
  device, path or workspace that kind names, checked per kind. A target switch
  compares and sets these columns, so it commits only against the selection it
  expected. They are not foreign keys: a conversation that targets a device
  directly does not block revoking that device. The full agent checkpoint
  belongs to the conversation database. `conversation_hosts` stores attached
  devices with a name unique within the conversation and their last cwd.
  `conversation_panels` stores each conversation's
  [work panel state](../product/web-api.md#work-panel-state) as one JSON
  document, replaced whole by every save and deleted with its conversation.
- **Operations:** `conversation_fork_operations` reserves a destination ID and
  records source boundary, owner, target, full model selection, title,
  creation time, and attached hosts. `managed_operations` records reset intent
  and progress under device and operation IDs. These records make interrupted
  multi-step work discoverable; they are not cross-database transactions.
- **Providers:** `providers` stores owner, family, credential kind, label,
  an API-key entry's sealed configuration, and a subscription entry's active
  account; a subscription entry has no configuration, since its credentials
  are its accounts. `provider_credentials` stores one subscription account per
  row: entry, identity key (unique within the entry), label and detail, how
  the account arrived, the sealed secret document, a version that every secret
  write advances, the account's usage snapshot, and the time of its last
  write. Writing an account into an entry that has no active account selects
  it in the same transaction. Rows are removed with their entry. A partial
  unique index enforces one subscription entry per owner and family. The owner
  is always a user: a shared instance's entries are the master's.
  `model_catalogs` stores one validated cache record per provider entry, the
  catalog with its key and the time of its last check, removed with that
  entry. See
  [Providers](../providers/providers.md#credential-vault) and
  [Models](../providers/models.md#catalog-cache).
- **Usage and attachments:** `usage_ledger` stores user, conversation,
  provider, model, token counts, and observation time; when a row is written
  and what the ledger promises are defined in
  [Usage and quota](../providers/usage-and-quota.md#usage-ledger).
  `attachments` stores owner, media type, byte length, content hash, and
  creation time. Neither table contains the attachment bytes.

Conversation and workspace `sort_order` represent explicit user ordering;
activity timestamps do not reorder them. A read acknowledgement advances
`read_revision` with `MAX`, so a browser that holds a stale revision cannot
move it backward. The backend refuses a revision beyond current output.

Projects retain their explicit order; conversations are ordered within their
project and pin partition. New projects append and new conversations enter at
the front. Rename, archive/restore, and target changes retain sort positions,
with ID as the stable tie-breaker. Reorder writes one partition atomically and
rejects archived rows or cross-partition targets.
[Web API](../product/web-api.md#sidebar-mutations-read-state-and-page-synchronization)
owns the request contract.

## Conversation state and transactions

Each `conversations/<id>.sqlite` contains one agent tree. Its root node ID is
the conversation ID; child nodes are subagents. The backend supplies this
database to the agent through the tree store contract
([Tree store](../agent/runtime.md#tree-store)), independently of any execution
Host. The node lifecycle and its commits are defined in
[Subagents](../agent/subagents.md#persistence).

| Table | Meaning |
|---|---|
| `nodes` | Parent relationship, description and profile, spawn time and whether the node may spawn children, close result or failure, completion-delivery state, checkpoint state, block count, command and output revisions |
| `blocks` | One transcript block per node and block index |
| `command_snapshots` | Immutable complete command-state maps indexed by node and revision ([Command state history](../agent/command-state-history.md)) |
| `session_boundaries` | History cutoffs linked to a command revision |

For example, saving a streamed assistant block updates that block's row and
the node checkpoint together. It does not serialize the whole transcript into
the node's state. Rewinding history deletes rows beyond the new block count
and restores the corresponding command state. The journal is therefore a
sequence of indexed blocks, not an append-only database.

Each atomic commit of the tree store, such as creating, saving or closing a
node ([Persistence](../agent/subagents.md#persistence)), is one transaction on
the conversation's writer connection. Deleting a node cascades to its
descendants and their dependent rows.

A save publishes the checkpoint's media before its transaction
([Attachment and transcript media](#attachment-and-transcript-media)). The
transaction commits the block rows, the state and the command state together
or rolls all three back, so a crash at any moment leaves one complete
checkpoint. What a save carries, its order, and the guard it checks right
before its transaction belong to the tree store contract
([Saving](../agent/runtime.md#saving)).

Changed output advances `output_revision` in the checkpoint transaction; user
input alone does not. A summary read takes the phase, the output revision and
the latest terminal block without loading the transcript; a conversation that
has no database file yet reads as idle with revision 0. The user's shard
combines these persisted facts with live activity to produce browser
summaries.

The conversation stores hand out one stable handle per conversation database.
A handle opens its writer connection on demand, on a thread of its own; at
most 64 writer connections are open at once, and opening another closes the
least recently used. Closing a connection loses nothing; the next operation
opens it again. A read that needs no live session, such as a summary, cold
history, or the files a command edited, uses a short-lived read-only
connection on the blocking pool instead. It takes no writer slot and never
creates a database file, so a state poll that reads the summaries of hundreds
of conversations does not close the writers of running sessions. Backend
shutdown closes every connection.

## Attachment and transcript media

The blob store selects a namespace by user: the authenticated user's for
uploads and downloads, and, for transcript persistence, the conversation
owner's, which the user's shard knows. A content hash identifies bytes only
within that namespace. Knowing another user's hash grants no access to their
blob.

The tree store moves the inline media of root and subagent blocks into the
owner's namespace before it saves them, and puts the bytes back when a session
loads for inference; cold browser history keeps the references. The agent
defines that mapping, including what a missing blob becomes
([Media](../agent/runtime.md#media)); the backend decides where the bytes go.
Browser delivery and uploaded attachment resolution are defined in
[Backend media handling](backend.md#media-by-reference).

Blob publication precedes the database checkpoint that references it. A
database failure can leave an unreferenced blob; it must not leave a committed
block pointing at an incompletely published upload. There is no transaction
spanning SQLite and the object store. Nothing reclaims an unreferenced blob;
when one may go is an open decision ([Open decisions](#open-decisions)).

## The object store

Blobs and the change store share one object store, in two key namespaces:

```text
blobs/<userId>/<sha256>           attachment and transcript media bytes
changes/<conversationId>/...      both sides of every file a command edited
```

A single-backend deployment keeps the object store in its data directory;
`DEMI_CHANGE_STORE_CONFIG` puts it in an S3 bucket, which the multi-worker
deployment requires. The backend reaches both through the `object_store`
library, so one code path serves a local directory and S3, and conditional
creation and checksums come from the library.
[Edit tracking](../execution/edit-tracking.md#the-change-store) defines what
the change store holds and when it is written.

A blob put hashes the bytes with SHA-256 on the blocking pool, since a 25 MiB
upload would hold an async thread for tens of milliseconds, and then creates
the object only if its key does not exist. Finding the key already present is
success: the same key always names the same bytes, so repeated uploads of one
file store it once. A get of a malformed hash or of a missing object returns
absent; any other error propagates.

`DEMI_CHANGE_STORE_CONFIG` names a JSON file with `bucket`, `region`, an
optional HTTPS `endpoint` for an S3-compatible service, and an optional
`forcePathStyle`, false by default, which names the bucket in the request path
instead of the host name.
Credentials come from the standard AWS environment variables, a web identity
token, or container or instance metadata; shared credentials and profile
files are not read. Shutdown releases the storage client.

## Encodings and digests

Every stored value has one encoding, fixed by its column or by its type:

- **SQL columns hold scalars.** A time is an integer count of milliseconds
  since the Unix epoch, so ordering and comparison are exact. An identifier is
  text, stored exactly as received: a conversation ID keeps the case the
  browser sent. Conversation IDs are nevertheless compared without case, in
  the conversation index and in Fork reservations, so no two conversations
  have IDs that differ only in case: each ID names a database file,
  `conversations/<id>.sqlite` with the ID in lowercase, and a file system may
  ignore case. A closed set, such as a role, a device kind or a title origin,
  is text that a CHECK constraint limits and the reader decodes into its type.
- **Structured values are JSON columns typed by their schema,** encoded by
  the convention of the browser wire
  ([Encoding conventions](../architecture/contracts.md#encoding-conventions));
  a time in JSON is an RFC 3339 string in UTC with millisecond precision. For
  example, a conversation forked at 14:13:20 UTC on 21 September 2026 has
  `created_at` 1790000000000 in its `conversations` row, while the Fork
  operation that created it records `"createdAt": "2026-09-21T14:13:20.000Z"`
  in its JSON metadata. Command storage values are the JSON values the command
  wrote; the store does not interpret them.
- **Credentials are sealed BLOBs**
  ([Passwords and credentials at rest](#passwords-and-credentials-at-rest)).

An error block's failure record keeps a vendor's answer in the form its
owner defines ([The failure record](../agent/failures-and-recovery.md#the-failure-record)).

Every read decodes and validates what it reads, as it would any input from
outside the process
([Validation at entry](../architecture/contracts.md#validation-at-entry)). A
column value outside its set, a JSON value that does not match its type, or a
sealed value that does not open is corrupt: the read fails with an error that
names the table and column, and nothing repairs, replaces or defaults the
value. The tree store reads a checkpoint the same way: its model selection,
transcript blocks, command state, queued input, scheduled wakeups and edit
receipts are decoded into their types, and corrupt data stops the restore.

A digest of a JSON value is SHA-256 over its RFC 8785 canonical form, which
sorts keys and writes each number one way, so equal values have equal digests
whatever produced their text: `{"b":1.0,"a":2}` and `{"a":2,"b":1}` have the
same digest. Two digests are stored, both in hexadecimal:

- An edit's receipt, in the node's checkpoint, holds the digest of the edit
  request, by which a retried edit is recognized
  ([Message editing](../agent/message-editing.md)).
- A model catalog record's key is the digest of its entry's configuration and
  active account ([Models](../providers/models.md#catalog-cache)).

A blob's name is the SHA-256 of its bytes in lowercase hexadecimal.

## Passwords and credentials at rest

The control database holds no password, token or provider credential in a
form that works as stored:

| Secret | Stored as | Where |
|---|---|---|
| Account password | argon2id hash as a PHC string | `users`; a copy at issue in `email_challenges` |
| Web session token | SHA-256 of the token | `web_sessions` |
| Device token | SHA-256 of the token | `devices` |
| Email-change code | HMAC-SHA256 under a key derived from the instance secret | `email_challenges` |
| API-key entry configuration, with its key and endpoint | Sealed | `providers` |
| Subscription account secret | Sealed | `provider_credentials` |

A PHC string records the algorithm and its parameters beside the salt and the
hash, for example `$argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>`, so a stored
hash stays verifiable if the parameters change. New hashes use the argon2
crate's default parameters, which are OWASP's minimum configuration for
argon2id: 19 MiB of memory, two passes, one lane.
[Authentication and ownership](backend.md#authentication-and-ownership)
describes how hashing is scheduled and how a login for an unknown address is
verified.

A sealed value is AES-256-GCM ciphertext stored as one BLOB: a random 12-byte
nonce, the ciphertext, and the 16-byte tag. The plaintext is a typed JSON
document: an API-key entry's configuration, or a subscription account's
secret document, which is one strictly typed document per family
([Subscription secrets](../providers/providers.md#subscription-secrets)). The
additional authenticated data names the row the value belongs to: the entry
for a configuration, the entry and the account for a secret. A sealed value
copied into another row therefore does not open. A value that does not open,
because it was altered, moved, or sealed under another instance secret, is
corrupt: the operation fails and nothing replaces the value.

The keys come from the instance secret, 32 random bytes written as 64
hexadecimal digits. HKDF-SHA256 derives one subkey for sealing and another for
email-change codes, each under its own label, so neither key can stand in for
the other. The secret is `DEMI_INSTANCE_SECRET` when that is set; otherwise it
is the file `instance-secret` in the data directory, created on first start
and readable only by its owner (mode 0600). A malformed secret stops startup.

Sealing protects credentials when the database leaks alone; it is not key
management, and whoever holds both the database and the secret can open every
credential. A deployment sets `DEMI_INSTANCE_SECRET` when several workers must
share the secret, or to keep the secret apart from the data it protects. The
backend uses the RustCrypto implementations of these standard formats
(`argon2`, `aes-gcm`, `hkdf`, `sha2`, `hmac`).

## Multi-worker storage placement

The multi-worker deployment adds one internal control service and keeps the
same SQLite schemas and ownership boundaries:

```text
Worker A                             Worker B
+-----------------------------+      +-----------------------------+
| Owned conversation DBs      |      | Owned conversation DBs      |
| Owned managed machines      |      | Owned managed machines      |
| control service client -----+--+ +-+----- control service client |
+-----------------------------+  | | +-----------------------------+
                                 v v
                      +-----------------------+
                      | control service       |
                      | control.sqlite        |
                      +-----------------------+

Workers: blob and change objects -> S3
Each SQLite owner: database replication -> S3
```

Workers serve the public API and own their assigned users' live state. Only
the control service opens `control.sqlite`; it never opens conversation
databases or machine disks. A single-backend deployment runs the same control
operations in process. [Backend deployment](backend.md#deployment-and-user-ownership)
owns routing and user movement.

Each control operation takes owned, serializable input and runs as one
transaction, so a worker calls the control service with one private,
service-token-authenticated `POST /rpc/<operation>` JSON request per
operation. Domain errors carry `{ code, message }`. SQL and transaction
handles never cross the wire. Authentication lookups can use a short-lived
worker cache; conversation block writes remain worker-local.

Independent usage and conversation-index updates do not commit atomically
with a transcript. Operations that require recoverable publication, such as a
fork, must use their recorded intent and completion checks. There is no
general transaction spanning the control database, conversation databases,
and files.

The selected replication approach is asynchronous SQLite replication to S3
using Litestream. It is optional for a local deployment and required for
multi-worker recovery. Recovery restores a snapshot and subsequent replicated
log data after fencing the stale writer. The recovery point is the last
successfully replicated state; an unhealthy replicator can lose more than one
configured sync interval.

Blobs and change objects live in the S3 object store, which every worker
reaches. A user's Cloud disks need their own restoration path: moving a user
to another worker publishes the machine's disk generation where the
destination can restore it. A generation kept in object storage must stay
atomic, with its manifest updated conditionally after both images are
stored; two image objects uploaded independently are not a committed
generation. The machine manager owns generations
([Save a generation](../cloud/managed-hosts.md#save-a-generation)). All
workers that need provider configuration share the instance secret through
deployment configuration.

## Open decisions

These durability questions are open. Each must be decided before the behavior
that depends on it is built. The second and third also gate the multi-worker
deployment, and the [Roadmap](../delivery/roadmap.md#decisions-before-expanding-deployment)
lists them with its other decisions; this section describes what each one
means for stored data.

- **Local object durability.** A save writes an uploaded image into the local
  object store, then commits the checkpoint that references it. If power fails
  after the commit but before the operating system has written the object's
  data, the committed block can point at a missing or truncated object,
  although publication preceded the checkpoint in the process. Whether a local
  publication syncs file and directory data before it counts as published is
  undecided. Until it is, blobs do not claim the durability of committed
  machine generations, which sync before they are published.
- **Fencing and user movement.** A worker can lose its route while it still
  runs. If the destination restores the replicated conversation databases
  while the stale worker still writes a checkpoint or a disk, the two diverge.
  Multi-worker recovery needs a definition of how the stale worker is fenced
  and of when a replicated checkpoint is ready for the destination to open.
- **Retention.** Nothing deletes a blob that no block references, such as an
  attachment edited out of history or one published by a save that then
  failed, or the blobs and change objects of a deleted account. A retention
  policy must say when such data may go, and a collector must not race a save
  that has published a blob but not yet committed the checkpoint that
  references it.
