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
  overrides, the last explicit model settings for new conversations, and the
  locale the browser last reported. A patch merges specified fields in one
  transaction so independent edits do not overwrite each other.
- **Devices and workspaces:** `devices` stores ownership, kind, name and
  platform, the hash of the device's current token, and claim and last-seen
  times. The token hash is unique, so a runner's token finds its device
  through one index lookup; it is absent until the backend issues a token. A
  partial unique index permits one managed device per user. A user's paired
  devices are listed by claim time, oldest first, and two claimed within one
  millisecond by id. `workspaces` names directories on devices; deleting a
  workspace never deletes its files. Online status comes from live
  connections, not the last-seen time. `exposes` stores
  each [Host expose](../execution/expose.md#the-expose-record): id, owner,
  device, target address, creation and expiry time. Expiry, removal, a Cloud
  stop and device revocation delete rows; nothing updates a row except
  renewal.
- **Conversation index:** `conversations` stores ownership, title and its
  [origin](../product/product.md#conversation-titles), archive and pin state,
  ordering, read revision, target selection, target context revision, last
  switch, Cloud reset marker, the conversation's model selection as JSON
  ([A conversation's model settings](../providers/models.md#a-conversations-model-settings)),
  the counts of messages the user sent and the last generated title had
  seen, and when its agent tree was last seen live (`live_at`), from which
  the retention pass reads how long the conversation has been idle
  ([Retiring tool media](#retiring-tool-media)). The target is
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
  `conversation_drafts` stores each conversation's
  [draft](../product/web-api.md#conversation-drafts): its revision, its text
  and files as one JSON document with the revision they were written at, and
  the replaced version as another, or null. A save reads the row and writes it
  in one transaction, so two saves never build on the same revision. The row
  stays when the draft is emptied, so a revision is never used twice, and it
  is deleted with its conversation; the conversation index reads the revision
  into each conversation's summary.
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
  `attachments` stores owner, media type, byte length, content hash, a text
  file's snippet, and creation time. Neither table contains the attachment
  bytes.

Conversation and workspace `sort_order` represent explicit user ordering;
activity timestamps do not reorder them. A read acknowledgement advances
`read_revision` with `MAX`, so a browser that holds a stale revision cannot
move it backward. The backend refuses a revision beyond current output.

Projects retain their explicit order; conversations are ordered within their
project and pin partition. New projects append and new conversations enter at
the front. Rename, archive/restore, and target changes retain sort positions,
with ID as the stable tie-breaker. Reorder writes one partition atomically and
rejects archived rows or cross-partition targets.
[Web API](../product/web-api.md#sidebar-mutations-and-read-state)
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
| `media` | An index of the blobs the blocks reference, for the [retention pass](#retention): one row per reference, with the node, the block index, the blob, whether a tool result holds it, and the block's time. The rows are derived from the blocks, never written on their own: one function derives a block's rows, and every path of the tree store that writes a block, a save, a history rewrite, an edit, a Fork's seed and a retirement, replaces that block's rows with it in the same transaction. It indexes what SQLite cannot index inside a block's JSON |
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

Changed output advances `output_revision` in the checkpoint transaction; input
alone does not. Input is the user's messages and steers, and the context,
wakeup, agent-message and resume blocks the session writes before a request;
everything else a save changes, and rows a rewrite removes, is output. A
summary read takes the phase, the output revision and the latest terminal
block without loading the transcript; a conversation that has no database file
yet reads as idle with revision 0. The user's shard
combines these persisted facts with live activity to produce browser
summaries.

The conversation stores hand out one stable handle per conversation database.
A handle opens its writer connection on demand, on a thread of its own; at
most 64 writer connections are open at once, and opening another closes the
least recently used. Closing a connection loses nothing; the next operation
opens it again. A read that needs no live session, such as a summary, cold
history, or the files a command edited, uses a short-lived read-only
connection on the blocking pool instead. It takes no writer slot and never
creates a database file, so a page's snapshot, which reads the summaries of
hundreds of conversations, does not close the writers of running sessions. Backend
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
spanning SQLite and the object store. The retention pass deletes a blob that
nothing references ([Retention](#retention)).

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

## Retention

Demi keeps what a user made for as long as the account exists: uploads,
conversations and their messages; a conversation can be archived but not
deleted ([Conversations and projects](../product/product.md#conversations-and-projects)).
What goes by itself is what tools produced: a tool result's images and videos
after 30 days, the blobs nothing references any more, and a conversation's
job output on a Host once its Host resources are released.

For example, on 1 September an agent takes a screenshot with a shell command,
and the tool result references it as the blob `blobs/<user>/ab12…`. The
conversation compacts on 3 September, so the screenshot lies before the
root's last `compaction_boundary`, where no request replays it. The daily
retention pass does the rest:

```text
1 Sep   the screenshot enters: blobs/<user>/ab12…, referenced by a tool_call block
3 Sep   compaction: the block now lies before the root's last boundary
1 Oct   the pass retires it: the block holds a line of text in its place,
        and ab12… is recorded as used on 1 Oct
2 Oct   the pass collects: no row references ab12…, the object and its last
        use are older than 24 hours, so the blob is deleted
```

| What | Kept | Then |
|---|---|---|
| An upload: its record and its blob | For as long as the account exists; the user set their retention aside | Removed with the account ([Account deletion](#account-deletion)) |
| A tool result's image or video | 30 days, and longer while a request could still send it | Retired to a line of text ([Retired tool media](../agent/runtime.md#retired-tool-media)), then collected |
| Any other blob, such as a tool's screenshot in history an edit removed | While a block, a queued message or a pending steer references it, and 24 hours after its last use | Collected ([Collecting blobs](#collecting-blobs)) |
| A conversation, its rows and its change objects | For as long as the account exists | Removed with the account |
| A shell job's output on a Host | Until the conversation's Host resources are released | Removed by the conversation release, or when a Cloud stops ([Conversation release](../execution/resource-lifecycle.md#conversation-release)) |

### The retention pass

Each backend runs one retention pass a day for every user placed on it, which
in the single-backend deployment is every user: the first once it has started
and startup recovery has published each
Fork destination whose root committed
([Backend creation and retries](../agent/conversation-fork.md#backend-creation-and-retries)),
then every 24 hours. It hands the users' passes to their shards one after
another, and a user's pass takes two steps in the user's shard: it retires
the expired tool media of the user's conversations, then collects the user's
blobs. The pass runs in the shard because the conversations' live trees, file
gates and holds are there ([The user shard](../architecture/concurrency.md#the-user-shard)):
it can tell a live tree from a stored one and hold a conversation as a
transition does. It waits on the databases and the object store, never
holding up the shard's other work.

In the multi-worker deployment, each worker runs the passes of the users it
holds: only that worker reads their conversation databases, and the use
record the collector checks is that worker's. Moving a user must fence the
old worker's pass with its other writers
([Open decisions](#open-decisions)).

A pass costs about as much as the user has conversations and blobs, not as
much history as the conversations hold. Per user and day:

- **Retiring:** one read-only query per conversation on its `media` rows for
  tool media older than 30 days. A node's blocks are read only when it has
  some, and the conversation's writer connection is opened only when there is
  something to retire.
- **Collecting:** one listing of `blobs/<user>/`, which on S3 is one LIST
  request per 1,000 blobs; one query of the user's upload records; per
  conversation, one read-only read of its `media` rows and its nodes' state
  rows; and one delete per blob it removes.

### Retiring tool media

The agent defines which images and videos are retired and the text that takes
their place ([Retired tool media](../agent/runtime.md#retired-tool-media)).
The pass applies that rule to every node of each conversation whose tree is
not live:

1. It holds the conversation as a transition does: it reserves the
   conversation's file gate, and leaves a conversation whose gate is busy to
   the next pass. Every conversation socket frame, an `open` included, is
   handled under a lease of that gate, so while the pass holds it no tree
   opens, and frames wait as they wait for a transition
   ([How a conversation uses a device](../execution/sessions-and-targets.md#how-a-conversation-uses-a-device)).
   A reservation is not demand, so it restarts no idle window.
2. It checks that the conversation still has no live tree.
3. In one transaction on the conversation's writer connection, it finds the
   nodes whose `media` rows hold tool media older than 30 days, reads those
   nodes' blocks, and writes in place each block the rule changes. The
   transaction changes no node's state row, block count or output revision,
   so the conversation does not show as unread.
4. It lets go of the conversation.

A live tree is left alone. Its sessions hold their transcripts in memory: a
save, a history rewrite or an edit would write the original block back, and
a changed block would change the history a page's editor is built on. The
pass records the tree as live instead. When a tree is disposed, the shard
retires its conversation at once, in the same four steps, unless the shard is
closing; the agent server tells it through `ServerDeps::status_changed`. A
conversation that stays open in a page is therefore retired within about 10
minutes of the last page leaving it
([Connections and the live tree](../agent/runtime.md#connections-and-the-live-tree)).

A conversation has been idle for 30 days when its tree is not live and its
`live_at` is at least 30 days old. The index sets `live_at` when it creates
the conversation, and the backend writes it again when the conversation's
tree becomes live, before the tree admits any action; when the tree is
disposed; and in each pass that finds the tree live. Only a live tree sends
requests, so once the tree is disposed no request is later than `live_at`.
After a crash there was no disposal, and the last request can be up to a day
later than `live_at`: the idle rule then applies 29 days after the last
request at the earliest, still far beyond any vendor's cache.

### Collecting blobs

The collector deletes a blob only when nothing can reach it. It reads two
kinds of evidence:

- **References:** the user's upload records, which hold every upload's hash,
  the files a draft stages included, since a draft names only uploads and
  files on devices ([Conversation drafts](../product/web-api.md#conversation-drafts));
  and, for every conversation of the user, archived ones included, its
  `media` rows, which cover every block of every node, and the media of each
  node's queued messages.
- **Uses:** what no row shows yet, such as a medium that was put but whose
  block is not saved, or a pending steer, which lives only in its session
  ([Pending steers](../agent/runtime.md#pending-steers)). Each backend
  records, per user and blob, when the blob was last used: when a put of it
  starts, before it asks whether the blob exists, and when a commit writes or
  removes a reference to it, inside the commit's transaction, before it
  commits. Each change of a conversation's `media` rows is such a commit. A
  use is remembered for 24 hours.

The object's age alone would not be enough: a put that finds its blob sends
nothing, so the object keeps its old time ([The object store](#the-object-store)),
and a Fork writes references it never put. The use record covers both, and
the recorded use of a retired medium keeps its blob for a day after the
retirement, longer than any copy of the rows that still named it takes.

The collector fails closed. When it cannot read one of the user's reference
sources, a conversation database that does not open or does not answer, or
the upload records, the collection deletes nothing for that user and logs
which source failed and why; the next pass tries again. Deleting on partial
evidence could remove a blob that the unread source still names.

A collection lists the user's namespace and keeps the objects older than 24
hours, drops every one a reference names, and then, for each blob left, does
one step on the use record: it checks that the blob's last use is older than
24 hours and marks the blob as being deleted. It then deletes the blob and
removes the mark. A put of a blob that is being deleted waits for the
deletion and then stores its bytes again, so a put never reports a blob that
is gone. A commit that would write a reference to such a blob fails instead
of committing; only a copy of rows that took longer than the grace could make
one, and none does.

### Crashes

No crash leaves a block that names a deleted blob:

- A retirement is one transaction per conversation. A crash leaves either the
  original blocks, which still reference their blobs, or the retired ones,
  whose blobs have not been collected yet.
- The collector deletes only blobs that no committed row referenced when it
  read the references, every reference source included, and that nothing used
  for 24 hours. A crash between two deletions leaves blobs that the next pass
  deletes.
- The use record is lost with the process, and so is everything it protected:
  a session's unsaved blocks, an upload in flight, a Fork's capture. Startup
  recovery publishes a committed Fork destination before the first pass reads
  references.

### Account deletion

There is no account deletion
([Account API](../product/web-api.md#account-api)). When it comes, it must
remove, besides the account's records and conversation databases, the objects
nothing else removes: the account's blob namespace, `blobs/<userId>/`, and the
change objects of each of its conversations, `changes/<conversationId>/`. Its
Cloud's disks belong to the machine manager
([Lifecycle and capacity](../cloud/managed-hosts.md#lifecycle-and-capacity)).

### Acceptance

The tests count at the object store with the backend's counting store and move
time on the test clock; none waits for a day to pass.

| Situation | Required result |
| --- | --- |
| A blob no row references, older than 24 hours and unused for 24 hours | The pass deletes it |
| A blob a block references; a blob put an hour ago; an upload's blob; a draft's staged file | The pass keeps each |
| A put of a blob the collector is deleting | The put waits and stores the bytes again; the block that names the blob can read it |
| A 31-day-old tool image before a boundary older than a day, in a stored conversation | It is retired; its blob goes at the next pass |
| A 31-day-old tool image in the replayed window of a conversation idle for 30 days | It is retired, and the conversation's next request carries its text |
| The same conversations while a page has them open | Nothing is retired until their trees are disposed |
| A message's image, 31 days old | Never retired |
| A crash inside a retirement or between two deletions | Every block's blob is still readable |
| One of the user's conversation databases does not open | The collection deletes nothing for that user and logs which database failed |
| After a save, a history rewrite, an edit, a Fork's seed and a retirement | Each conversation's `media` rows equal the rows derived from its blocks; the check fails when any of these paths skips the one function |

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
that depends on it is built. The second also gates the multi-worker
deployment, and the [Roadmap](../delivery/roadmap.md#decisions-before-expanding-deployment)
lists it with its other decisions; this section describes what each one
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
  The fence also ends the stale worker's retention pass, whose collector
  trusts only its own process's record of blob uses
  ([Collecting blobs](#collecting-blobs)).
