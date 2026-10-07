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
+-- search/<userId>.sqlite         each user's search index, derived from the conversations
+-- blobs/<userId>/<sha256>        user-owned bytes: uploads, media, edit copies, commands' outputs (object store)
+-- native/                        the published command packages (object store)
+-- .attributes/                   the local store's object attributes
+-- instance-secret                seals credentials, unless configured
+-- snapshots/<version>/           the databases as an upgrade from that release found them
```

The names identify storage responsibilities; deployment options supply the
actual roots, and `blobs/` and `native/` move to an S3 bucket when the
deployment's store is one ([The object store](#the-object-store)).
`snapshots/` belongs to `demi-server`, which keeps there the copy that a
[rollback](../delivery/upgrades.md#rollback) restores; the backend never
reads it. The `storage` module owns the databases and the object store. The `vault` module owns
credential records and access; credentials are control records, never files.
The machine manager keeps each Cloud's disk generations in its own data
directory ([Managed hosts](../cloud/managed-hosts.md#provisioning)); the
backend stores only the Cloud's device record and its reset intent. Working
project files belong to execution devices and are reached through the
conversation's host access; they are not conversation database content.

| Store | Owns | Writer |
|---|---|---|
| `control.sqlite` | Accounts, auth sessions, preferences, subagent settings, devices, workspaces, conversation index, providers, model catalogs, usage, attachment metadata, operation records, the users' plugin choices, plugin values and Host directories, conversations' permission requests and grants | The control service, on its database thread |
| Conversation database | Root and subagent nodes, checkpoint state, transcript blocks, command history, the records of commands' outputs | The shard of the user who owns the conversation |
| Search index | Titles and message text of a user's conversations, derived from them | The backend's indexer, one per user |
| User blob namespace | Uploaded bytes, transcript media, edit copies, commands' whole outputs and the files plugins keep, addressed by content hash | The upload route, the conversation socket when an uploaded image enters fitted, a session when a tool's medium enters its transcript, and the backend when a command ends |

Both kinds of database are SQLite, reached through rusqlite with SQLite
compiled into the executable, so a deployment needs no system SQLite. A
transaction is a synchronous closure on its connection's own thread, so
nothing asynchronous happens inside one. Databases use WAL, foreign keys and a
five-second busy timeout, and stay plain SQLite files that Litestream can
replicate ([Multi-worker storage placement](#multi-worker-storage-placement)).
Product modules call the control service's operations rather than issuing
control SQL. This document defines data meaning and atomicity; the SQL lives in
the storage module's schema.

### Schemas and migrations

Each kind of database has its schema, a SQL text, and the history of the
schemas that published releases shipped before it, each with the migration
that leads from it to the next. A schema's version is a digest of its text: the
first 31 bits of its SHA-256, which a database records in SQLite's own
`user_version` field.

For example, 0.2.0 adds a column to the devices table. The control schema's
text changes, so its version does too, and the history gains the schema that
0.1.0 shipped, with a migration that adds the column. A 0.2.0 backend that
opens 0.1.0's control database finds 0.1.0's version in the history and
applies that migration.

Opening a database does one of these, in one transaction:

| The database records | Opening it |
| --- | --- |
| No version, and holds no table | Applies the schema to the new database and records its version |
| The schema's version | Opens it as it is |
| A version in the history | Applies each migration from that version to the schema's, in order, and records the schema's version |
| Anything else | Stops the open, with an error that names the file |

Anything else is a database that a newer release made, after a rollback that
did not restore its snapshot, or one a development build made. The error says
which: a version newer than every one this release knows names the
[rollback](../delivery/upgrades.md#rollback) that restores the databases of
this release. A migration that fails rolls its transaction back, so the
database stays at the version it had, and the open fails with the step that
failed.

A migration is SQL, or a Rust function on the transaction where SQL cannot
express the change, such as re-encoding a stored value; what it writes goes
through the same encoding and validation as any write. The control database
is migrated as the backend starts, before it serves
([Startup and shutdown](backend.md#startup-and-shutdown)); a conversation's
database when its conversation is next opened, so a start does not wait for
every conversation a server ever had.
The [upgrade](../delivery/upgrades.md#the-upgrade) copies every database
that will migrate before the new release first opens it.

A migration is tested once: a database of each schema in the history,
migrated, has the same tables, columns and indexes as a new one, and a
migration that rewrites values is tested on its rows.

Every published release's schema is in the history, the releases before the
first formal one included, so that migrations are written and tested on real
upgrades, such as a server going from 0.1.11 to the next release, long before
a user depends on them. The first formal release clears the history: its
schemas start a new one, and the pre-release migrations go. A database that
records another version, such as one a development build of an unpublished
schema made, stops the open.

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
  overrides, the last explicit model settings for new conversations, where
  New Project last pointed, the locale the web app last reported, and the
  context limit the user set on each model
  ([Context limit](../providers/models.md#context-limit)). A patch merges specified fields in one
  transaction so independent edits do not overwrite each other.
- **Subagents:** `user_subagents` stores the
  [Subagent switch](../agent/subagents.md#profiles) of each user who turned
  it off or on again: user and whether subagents are on. A user with no row
  has them on. `subagent_profiles` stores each user's
  [subagent profiles](../agent/subagents.md#profiles), one per row: id, user,
  name, unique among the user's, description, the model settings as JSON or
  null for the parent's, the replacing instructions or null for the
  parent's, whether its children may spawn, and whether it is enabled, which
  a new row is. A patch merges the fields it names in one transaction, and
  checks the name's uniqueness in the same one.
- **Devices and workspaces:** `devices` stores ownership, kind, name,
  platform, the operating system and architecture and the runner release its runner last reported, the hash of the device's current token, claim and last-seen
  times, and the JSON list of artifacts its runner last reported its cache
  holds ([Installed artifacts](../execution/native-runtime.md#installed-artifacts)).
  The token hash is unique, so a runner's token finds its device
  through one index lookup; it is absent until the backend issues a token. A
  partial unique index permits one managed device per user. A user's paired
  devices are listed by claim time, oldest first, and two claimed within one
  millisecond by id. `workspaces` names directories on devices; deleting a
  workspace never deletes its files. Online status comes from live
  connections, not the last-seen time.
- **Conversation index:** `conversations` stores ownership, title and its
  [origin](../product/product.md#conversation-titles), archive and pin state,
  ordering, read revision, target selection, target context revision, last
  switch, Cloud reset marker, the conversation's model selection as JSON
  ([A conversation's model settings](../providers/models.md#a-conversations-model-settings)),
  the counts of messages the user sent and the last generated title had
  seen, and when the earliest yield
  wakeup its tree saved is due (`wakeup_at`), at which the next start
  restores the tree ([Yield wakeups](../agent/runtime.md#yield-wakeups)).
  `wakeup_at` is milliseconds since the Unix epoch, 0 for a wakeup whose
  action had not ended, which is due at start, and null when the tree saved
  none. The user's shard writes it after a commit of the tree that changed
  it, from the `nodes` rows ([Conversation state and
  transactions](#conversation-state-and-transactions)). The target is
  typed columns: its kind (Cloud, a device directory, or a workspace) and the
  device, path or workspace that kind names, checked per kind. A target switch
  compares and sets these columns, so it commits only against the selection it
  expected. They are not foreign keys: a conversation that targets a device
  directly does not block revoking that device. The full agent checkpoint
  belongs to the conversation database. `conversation_hosts` stores attached
  devices with a name unique within the conversation and their last cwd.
  `conversation_panels` stores each conversation's
  [work panel tabs](../product/web-api.md#work-panel-state): the revision,
  the tabs as one JSON document, and the ids the panel ever had. Each change
  reads the row and writes it in one transaction, so the changes of one
  conversation apply one at a time, and the row is deleted with its
  conversation.
  `conversation_drafts` stores each conversation's
  [draft](../product/web-api.md#conversation-drafts): its revision, its text
  and files as one JSON document with the revision they were written at, and
  the replaced version as another, or null. A save reads the row and writes it
  in one transaction, so two saves never build on the same revision. The row
  stays when the draft is emptied, so a revision is never used twice, and it
  is deleted with its conversation; the conversation index reads the revision
  into each conversation's summary.
  `permission_requests` stores each conversation's
  [permission requests](../agent/permissions.md#requests): id, conversation,
  category, the command line, the agent that ran it (its node, number and
  description, or the root), when it was raised, and, once decided, the
  decision. `permission_grants` stores each grant: conversation, category and
  when it was granted, one row per conversation and category. Allow writes
  the grant and decides the category's requests of the conversation in one
  transaction; a new request is written with the removal of the request it
  replaces; a decided request is deleted once its message is in the agent's
  checkpoint, and the requests still decided at start are delivered then; an
  archive deletes the conversation's requests.
  The conversation index counts the undecided requests into each
  conversation's summary.
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

- **Plugins:** `plugin_values` stores each plugin's values for a user
  ([The contract](../architecture/plugins.md#the-contract)): user, plugin id,
  key, the JSON document, its revision, and the SHA-256 of each blob the value
  names. A write names the revision it read and commits only if the row still
  has it, in one transaction, so two writes never build on the same revision;
  a value's first write expects none. `user_plugins` stores each choice a
  user made about a plugin: user, plugin id and whether it is on. A plugin
  with no row is on. `plugin_directories` stores each
  plugin's [Host directories](../architecture/plugins.md#host-directories) for
  a user: user, plugin id, the directory's name, its digest and its listing,
  each file's path, whether it is executable, and its SHA-256. A plugin's set for a user is replaced
  whole in one transaction. The plugin host decodes a value only as JSON; the
  plugin decodes it into its own type and refuses one that does not fit, as
  every reader of a stored value does.

Conversation and workspace `sort_order` represent explicit user ordering;
activity timestamps do not reorder them. A read acknowledgement advances
`read_revision` with `MAX`, so a page that holds a stale revision cannot
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
| `nodes` | Parent relationship, the agent's number and its current round ([Identifiers the model sees](../agent/runtime.md#identifiers-the-model-sees)) with the round's start time, description, its profile's name and the instructions the profile replaced ([Persistence](../agent/subagents.md#persistence)), whether the node may spawn children, close result or failure, completion-delivery state, checkpoint state, block count, output revision, and when the earliest wakeup the checkpoint state saves is due (`wakeup_at`, encoded as the index of conversations encodes it), which each save writes with the state, so the conversation's earliest wakeup is the least over its nodes; a root whose last turn was interrupted has none, since its wakeups wait for the user to resume it ([Yield wakeups](../agent/runtime.md#yield-wakeups)) |
| `sequences` | The next number of each sequence the model sees in the conversation: commands, shells, agents, conversation browser tabs and attachments. The backend advances a sequence in its own transaction before it gives the number out, by the count a native service asks for when it reserves several ([Conversation numbers](../execution/native-runtime.md#conversation-numbers)), so a crash leaves a gap and never gives a number twice |
| `blocks` | One transcript block per node and block index |
| `command_outputs` | The record of each ended command's whole output, by command id ([Command outputs](#command-outputs)) |
| `attachments` | Each attachment the agent uploaded, by its number: the file's name, its media type, its size and its blob ([Attachment commands](../execution/commands.md#attachment-commands)); a Fork's seed copies the rows, and the blobs stay shared |

For example, saving a streamed assistant block updates that block's row and
the node checkpoint together. It does not serialize the whole transcript into
the node's state. Rewinding history deletes rows beyond the new block count.
The journal is therefore a
sequence of indexed blocks, not an append-only database.

Each atomic commit of the tree store, such as creating, saving or closing a
node ([Persistence](../agent/subagents.md#persistence)), is one transaction on
the conversation's writer connection. Deleting a node cascades to its
descendants and their dependent rows.

A save writes its rows as they are: a block holds its media by reference,
and each blob was stored when its medium entered the transcript
([Attachment and transcript media](#attachment-and-transcript-media)). The
transaction commits the block rows and the state together or rolls both
back, so a crash at any moment leaves one complete checkpoint. What a save
carries and its order belong to the tree store contract
([Saving](../agent/runtime.md#saving)).

Changed output advances `output_revision` in the checkpoint transaction; input
alone does not. Input is the user's messages and steers, and the context,
wakeup, agent-message and resume blocks the session writes before a request;
everything else a save changes, and rows a rewrite removes, is output. A
summary read takes the phase, the output revision and the latest terminal
block without loading the transcript; a conversation that has no database file
yet reads as idle with revision 0. The user's shard
combines these persisted facts with live activity to produce the
summaries the web app receives.

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

Root and subagent blocks hold their media and edit copies by reference in
every row and every frame, so the conversation database holds none of their
bytes. Each is stored once, when it enters: an upload by the upload route, an uploaded image that
fitting changed ([Images in the transcript](../agent/runtime.md#images-in-the-transcript))
by the conversation socket as it resolves the message, a tool's medium by
the session, through its tree store, before the tool's result enters the
transcript, and an edit copy by the backend when its command ends
([Edit copies](../execution/edit-tracking.md#edit-copies)). A session reads back, through the same store, only the media its
provider requests send, and cold history for the web app reads none. The agent
defines these rules, including what a missing blob becomes
([Media](../agent/runtime.md#media)); the backend decides where the bytes go.
Delivery to the web app and uploaded attachment resolution are defined in
[Backend media handling](backend.md#media-by-reference).

A blob is published before any row or frame that references it: an upload
is stored before a message can name it, a fitted image before its message
reaches the session, a tool's medium enters the transcript only once its put
has succeeded, an edit copy is stored before the block that lists it, and a
command's output before its row. A database failure can leave an
unreferenced blob; it never leaves a committed block pointing at unpublished
bytes. There is no transaction spanning SQLite and the object store, and an
unreferenced blob stays ([Retention](#retention)).

## Command outputs

When a command ends, the backend stores its whole output, as its Host kept it
within 16 MiB, as a blob in the conversation owner's namespace
([The whole output](../agent/runtime.md#the-whole-output)), and records it in
the conversation's `command_outputs` table, one row per command, keyed by the
command's id. The row holds when the command ended and one of two states:

| State | Holds | `demi shell output` prints |
|---|---|---|
| Stored | The blob, and the bytes at the output's end that the backend does not have with why, when there are some: lost with the Host's connection, or not read from the Host; and the command's media ([Media a command returns](../agent/runtime.md#media-a-command-returns)), each with its number, media type and size, and its blob or why the backend does not have it: lost with the Host's connection, not read from the Host, or not stored | The output, and with `--medium` one medium |
| Not stored | Why the put failed | The reason |

The puts come first, the output's and each medium's, and the row after
them, as for every blob
([Attachment and transcript media](#attachment-and-transcript-media)): a
stored row never names an unpublished blob, and a crash between the two leaves
an unreferenced blob. A row is written once, when its command ends, and never
changes. No session holds the rows: `demi shell output` reads a row, then its
blob, on the conversation's read-only connection.

## The object store

A deployment has one object store, and it holds every object the backend
keeps: the users' blobs and the published command packages.

```text
blobs/<userId>/<sha256>                uploads, transcript media, edit copies and commands' outputs
native/blobs/<sha256>                  a command package's or the runner's executable
native/descriptors/<digest>.json       a command package's descriptor
native/packages/<id>/<version>.json    the descriptor a package version names
```

`DEMI_STORAGE` chooses where the store lives, and the two choices are
equal: the same keys, the same rules for creating and reading an object, and
the same publication of the command packages
([Publish packages, then source artifacts on demand](../execution/native-runtime.md#publish-packages-then-source-artifacts-on-demand)).

| `DEMI_STORAGE` | The store | How a runner downloads a command artifact |
| --- | --- | --- |
| `local`, the default | The data directory, each key a file beneath it | From the backend, at `GET /native-artifacts/<sha256>` on its public URL |
| `s3` | A bucket the `DEMI_S3_*` settings below name | From the bucket, through a URL the backend signs |

The one difference is where a runner downloads from: a file in the data
directory is reachable only through the backend, so the backend serves it.
The multi-worker deployment requires `s3`, since its workers share the store
([Multi-worker storage placement](#multi-worker-storage-placement)). The
backend reaches the store through the `object_store` library, so one code
path serves a local directory and S3, and conditional creation and checksums
come from the library. An object carries metadata, such as the SHA-256 and
size of a command artifact; S3 keeps it with the object, and locally it lies
in a file of its own under `.attributes/` in the data directory, written
before the object, since a file has no place for it. No listing of the
store shows those files.

A local object counts as written only once it is durable, as an S3 object is
when its PUT succeeds. The store writes the object and its attributes to
temporary files, syncs them, links them into place, and syncs their
directory. For example, a session writes a tool's image, then commits the
checkpoint that references it; if power fails right after the commit, the
image is on disk, so the committed block never points at a missing or
truncated object. Committed machine generations sync before they are
published in the same way
([Save a generation](../cloud/managed-hosts.md#save-a-generation)).

A blob put hashes the bytes with SHA-256 on the blocking pool, since a 25 MiB
upload would hold an async thread for tens of milliseconds, and then asks
whether the key exists: a HEAD request on S3, the file's metadata locally. A
key that exists is success, and no bytes are sent: the same key always names
the same bytes, so repeated uploads of one file store it once. Otherwise the
put creates the object only if its key does not exist, and a key that another
put created meanwhile is success too. On S3 a new blob therefore costs a HEAD
and a PUT, one round trip more than the PUT alone, and a blob that exists
costs the HEAD alone, where a conditional PUT would send the whole body, up to
25 MiB, before S3 refused it; locally, a put of a blob that exists writes no
staged copy of it. A get of a malformed hash or of a missing object returns
absent; any other error propagates.

With `DEMI_STORAGE=s3`, four settings name the bucket:

| Variable | Meaning |
| --- | --- |
| `DEMI_S3_BUCKET` | The bucket. Required with `s3`; refused with `local`, like the other three. |
| `DEMI_S3_REGION` | The bucket's region. Required with `s3`. |
| `DEMI_S3_ENDPOINT` | An HTTPS endpoint of an S3-compatible service. Optional: the region's AWS endpoint otherwise. |
| `DEMI_S3_FORCE_PATH_STYLE` | `true` names the bucket in the request path instead of the host name. Optional, `false` by default. |

The bucket belongs to the deployment: its keys are the ones above, with no
prefix. An S3-compatible service must support conditional writes (a PUT with
`If-None-Match: *`, which the conditional creation above sends), SHA-256
upload checksums (`x-amz-checksum-sha256`, which every upload carries and the
service verifies), object metadata and presigned GET requests.
Credentials come from the standard AWS environment variables, a web identity
token, or container or instance metadata; shared credentials and profile
files are not read. Shutdown releases the storage client.

## Retention

Demi keeps everything a conversation made for as long as the conversation
exists: its messages, the files the user sent in it, a tool result's images
and videos, a command's whole output and its media, edit copies and every
other blob. Nothing expires. Deleting a conversation removes it and the blobs
no other record of its user references
([Deleting a conversation](#deleting-a-conversation)); removing the account
removes the rest ([Account deletion](#account-deletion)). A blob a failed
write leaves without a reference costs storage until the next collection of
its user's namespace, never correctness, since no row names it.

### Deleting a conversation

The user deletes "Fix the login test", which uploaded a screenshot that a
fork of it also holds. Deletion goes in four steps:

1. One transaction of the control database removes the conversation's
   record and every record that belongs to it, such as its draft, work
   panel, attached hosts and permission requests and grants, and records the
   deletion as pending. The usage ledger keeps its rows: they are the
   user's record of what was used. From this
   commit on, no request finds the conversation.
2. The conversation's Host resources are released as archive releases them,
   and its tabs in the conversation browser are closed.
3. Its database file and its rows in the search index are removed.
4. The pending record is removed.

A start that finds a pending deletion finishes it from the step it reached,
so a crash in between leaves nothing of the conversation behind. Its
project's files stay: they belong to the Host, not to the conversation.

Then the user's blob namespace is collected, in the background: a blob that
no remaining record of the user references, and that was written more than a
day ago, is removed, and so is an upload record whose blob goes. A record
references a blob when it names it: a block, an attachment, a command's
output record, an edit copy or a message queued in a checkpoint's state in
one of the user's conversation databases, a fork's database still being
made, a draft, a plugin value's or a plugin Host directory's list of blobs,
or an upload record written in the last day. The day keeps a blob whose
reference is not written yet, since a blob is published before the row that
names it: an upload the composer holds before its draft is saved, or a
tool's image before its checkpoint commits. A put of a blob that exists
writes nothing, so the blob's own age is its first write's; the user's shard
therefore remembers, for a day, each blob put again, and the collection
keeps those too. A restart forgets them, and with them only references
that the stopped process had not written and never will. The screenshot stays, because the fork's
database names it. A user's collections run one at a time; a deletion during
one starts another after it.

### Account deletion

There is no account deletion
([Account API](../product/web-api.md#account-api)). When it comes, it must
remove, besides the account's records and conversation databases, the objects
nothing else removes: the account's blob namespace, `blobs/<userId>/`. Its
Cloud's disks belong to the machine manager
([Lifecycle and capacity](../cloud/managed-hosts.md#lifecycle-and-capacity)).

## Search index

Each user has a search index, `search/<userId>.sqlite`, that
[Search](../product/web-api.md#search) reads: an SQLite FTS5 table with one
row per searchable text, a conversation's title or the text of one of its
root's `user` blocks or answers, each with its conversation's and block's
ids. Its tokenizer is FTS5's `trigram`, which matches any piece of text of
three characters or more in every language without splitting words, as
Chinese needs; a query word of one or two characters is matched by a scan of
the user's rows instead. Searching a user's conversations without an index
would open every conversation's database for each query, and one index per
user follows the users' shards
([Multi-worker storage placement](#multi-worker-storage-placement)).

The index is derived: everything in it is read from the conversations, so it
is never migrated, backed up or replicated, and it is the one stored copy of
data that this document allows to be derived, for speed. Its version is the
digest of its schema, as for the databases; an index of another version, or
one that fails to open, is deleted and built again. For each conversation it
records the transcript version it indexed, the transcript's epoch and
revision ([Patches and versions](../agent/runtime.md#patches-and-versions)),
and the title. The backend indexes a conversation again, in one transaction
that replaces its rows, so a search sees it either as it was or as it is:

- after a checkpoint changes the root's transcript, at most once every two
  seconds per conversation, and once more when its turn ends;
- after a title change;
- at start, for each conversation whose recorded version or title differs,
  newest first, in the background, so a new index fills while the backend
  serves.

A deletion removes the conversation's rows.

## Encodings and digests

Every stored value has one encoding, fixed by its column or by its type:

- **SQL columns hold scalars.** A time is an integer count of milliseconds
  since the Unix epoch, so ordering and comparison are exact. An identifier is
  text, stored exactly as received: a conversation ID keeps the case the
  web app sent. Conversation IDs are nevertheless compared without case, in
  the conversation index and in Fork reservations, so no two conversations
  have IDs that differ only in case: each ID names a database file,
  `conversations/<id>.sqlite` with the ID in lowercase, and a file system may
  ignore case. A closed set, such as a role, a device kind or a title origin,
  is text that a CHECK constraint limits and the reader decodes into its type.
- **Structured values are JSON columns typed by their schema,** encoded by
  the convention of the web app's wire
  ([Encoding conventions](../architecture/contracts.md#encoding-conventions));
  a time in JSON is an RFC 3339 string in UTC with millisecond precision. For
  example, a conversation forked at 14:13:20 UTC on 21 September 2026 has
  `created_at` 1790000000000 in its `conversations` row, while the Fork
  operation that created it records `"createdAt": "2026-09-21T14:13:20.000Z"`
  in its JSON metadata.
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
transcript blocks, queued input, scheduled wakeups and edit
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

Workers: blobs -> S3
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

Blobs, edit copies among them, live in the S3 object store, which every worker
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

This durability question is open, and must be decided before the behavior
that depends on it is built. It also gates the multi-worker deployment, and
the [Roadmap](../delivery/roadmap.md#decisions-before-expanding-deployment)
lists it with its other decisions; this section describes what it means for
stored data.

- **Fencing and user movement.** A worker can lose its route while it still
  runs. If the destination restores the replicated conversation databases
  while the stale worker still writes a checkpoint or a disk, the two diverge.
  Multi-worker recovery needs a definition of how the stale worker is fenced
  and of when a replicated checkpoint is ready for the destination to open.
