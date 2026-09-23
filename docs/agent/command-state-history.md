# Command state history

## Contract

Command storage holds small, structured state owned by one agent node, such as
its todo list. Each committed mutation creates an immutable version. A
conversation boundary records the version that was current at that boundary.
Fork and transcript rewrites restore that version without executing historical
commands.

The agent runtime owns versioning for every command storage key. The coding
harness owns todo validation and operations; it does not implement its own
history. `todos.json` is a logical key, not a file. Files, processes,
credentials, caches, and other external effects do not become versioned
conversation state.

Command storage is the only structured state a node keeps that follows its
history; the harness keeps no state of its own. A todo command needs neither
the transcript nor any harness internals: it reads and writes its key through
storage messages ([Mutation API and concurrency](#mutation-api-and-concurrency)).

## Example

| Ordered action | Current command state | Recorded boundary |
| --- | --- | --- |
| Create a conversation | V0: empty | None |
| Start processing U1 | V0 | Before U1: V0 |
| Add T1 | V1: T1 pending | None |
| Finish assistant message A1 | V1 | After A1: V1 |
| Start processing U2 | V1 | Before U2: V1 |
| Complete T1 | V2: T1 done | None |
| Finish assistant message A2 | V2 | After A2: V2 |

A Fork after A1 starts with T1 pending. The source keeps T1 done. Editing U2
restores V1 before processing its replacement. Editing U1 restores V0. Updates
in either conversation afterwards create versions owned by that conversation.

## Immutable versions and boundary references

Each version stores the complete key-value map. Todo state is small and changes
infrequently; complete snapshots keep restore simple and avoid replaying a
mutation log. Large files and command output belong in files and the blob
store, not in this map.

In the conversation's database
([Storage](../backend/storage.md#conversation-state-and-transactions)):

- each version is one immutable row, keyed by node and revision, that holds the
  complete map;
- the node's row names its current revision;
- each boundary is one row that names the node, the block, the edge, and the
  revision.

Version identity is scoped to the node. V0 is the explicit empty initial
version. A new revision is one more than the largest revision the node has
used, so revisions stay unique within a node, including after a rewind. The
current revision can name an older version, so it is not inferred from the
largest revision. Reads use the selected version; no other writable copy of a
key exists anywhere.

A key is a non-empty relative name: it contains no NUL, does not start with
`/` or a drive letter, and has no `..` segment. A value is any JSON value.
Reads return copies; a handler changes storage only through a write message.
Versions and boundary references are validated where the store reads them. The
agent runtime validates keys and values; the todo command validates the todo
schema when it reads its key. An invalid or missing referenced version is an
error, not an empty list or a fallback to the source's latest state.

Versions remain for the lifetime of the conversation. A Fork copies the
versions its retained boundaries reference into the destination's database; it
does not depend on the continued existence of the source's database.

## When a boundary is recorded

The session records three kinds of boundary:

| Edge | Recorded for | Records |
| --- | --- | --- |
| `before_user` | A block that opens an input turn: a `user` block, or a `context` or new-turn `wakeup` block ([Transcript](runtime.md#transcript)) | The version current when the session started processing that turn, before preparation hooks or commands of the turn change state. Recorded once. |
| `after_assistant` | Assistant text, when it finishes | The version current at that completion. Recorded once. |
| `after_block` | Every block | The version current when the block last changed. It moves forward with the block. |

Queue admission alone does not establish a `before_user` boundary. These are
explicit transcript boundaries, not wall-clock comparisons, and not the time
the browser receives a message.

A command can update todos while a provider is streaming, or after a shell tool
returns with a background job still running. Therefore:

- Every successful command-state mutation commits immediately; it does not wait
  for the tool call or the model turn to end.
- Assistant completion and state commits share the node's persistence order. A
  commit ordered before the completion is included in that message's boundary;
  one ordered after it is excluded.
- Storage history is separate metadata. It does not insert transcript blocks
  between text deltas, split a displayed assistant message, or alter provider
  replay.
- Fork eligibility uses the recorded completion boundary. It does not infer
  completion merely because another internal event was appended.

Every operation that cuts the transcript restores a version recorded at the
cut, chosen by the same selector that determines the retained transcript:

- Editing restores the target's `before_user` version.
- Retry keeps the first input block of the last turn and restores that block's
  `before_user` version, or its `after_block` version when the turn began with
  an agent message.
- Resume, and an automatic retry of a failed provider request, restore the
  `after_block` version of the last retained block, or V0 when no block remains
  ([Failures and recovery](failures-and-recovery.md)).

Compaction does not replace structured command state with a model-generated
summary or discard versions that retained history references.

## Mutation API and concurrency

A command reaches its node's storage through the `rpc` port, where every
operation is a message, as is everything else an `rpc` handler receives
([The TypeScript boundary](../architecture/contracts.md#the-typescript-boundary)).
A callback cannot be a message, so an atomic update is a compare-and-set on the
node's revision, and handlers in the backend's own process use the same
messages:

| Message | Effect | Reply |
| --- | --- | --- |
| Read a key | Reads the key in the current version | The value, or none, and the node's current revision |
| List a prefix | Lists the keys that start with the prefix | The key names, sorted |
| Write-if a key | Sets the key to a value, or removes it, if the node's current revision is still the expected one; with no expected revision, the write is unconditional | Committed, or a conflict |

For example, two `demi todo add` commands, A and B, run concurrently on the same
node:

```text
A: read todos.json           -> [T1], revision 4
B: read todos.json           -> [T1], revision 4
A: write-if 4, [T1, T2]      -> committed, revision 5
B: write-if 4, [T1, T3]      -> conflict: the current revision is 5
B: read todos.json           -> [T1, T2], revision 5
B: write-if 5, [T1, T2, T3]  -> committed, revision 6
```

Both tasks survive and get distinct IDs, because B computes its ID from the
list it read last. An update is this loop: read, compute the new value, write-if
with the revision read, and on a conflict start again. The expected revision is
the node's, not the key's: a committed write to any key of the node makes a
concurrent compare-and-set conflict, and the handler reads again. The update's
computation performs no external IO, so repeating it is harmless.

Reads do not create versions. A write whose value equals the current one under
RFC 8785 canonical JSON, where key order and the form of a number do not
matter, creates no redundant version. Removing a key commits a version without
it.

Storage writes take their place in the session's one persistence order, with
checkpoint saves, boundary captures, and history rewrites
([Saving](runtime.md#saving)). A running turn never holds that order, so a
command's write waits only for the saves ahead of it, never for the rest of the
turn. The store commits a new version and the node's current revision in one
transaction. A transcript boundary and its version reference are committed
together, and a transcript rewrite commits the retained history and the
selected command state together. An older throttled checkpoint cannot overwrite
a newer state commit.

A command's success is returned after its storage transaction commits. Before
that point, cancellation or failure leaves the previous version current. After
that point, an output-pipe failure or a cancellation does not undo the
committed state; ordinary command reads observe it. A Fork sees either the
complete old state or the complete new state at its selected boundary, never a
partially written map.

The backend binds each job to its caller's storage when it starts the job: its
record holds the calling node and the node's current history generation
([Bind jobs to their caller](../execution/sessions-and-targets.md#bind-jobs-to-their-caller)).
The runner receives no storage binding, and nothing reads it from environment
variables. A job that `demi host shell` starts on another Host carries its
invoking job's binding. When a job's command sends a storage message, the
backend passes it to that node with that generation.

- A transcript rewrite or the node's disposal starts a new generation, so a
  late storage message from an older job cannot commit into restored history.
- The store checks the invocation's cancellation and the generation after
  writing media and immediately before its transaction.
- While an edit is being prepared, the node refuses storage messages
  ([Message editing](message-editing.md#admission)).
- Waiting writes fail when the node is disposed; the persistence order is
  released on success, failure, and cancellation.
- A Fork does not invalidate invocations in the source.

## Responsibilities

| Part | Responsibility |
| --- | --- |
| Command system | The storage messages on the `rpc` port: read, list, and write-if. It knows nothing about transcripts or todo fields. |
| Agent node | Owns each root's or child's command storage and the history generation its jobs are bound to. |
| Agent session | Orders state commits, captures message boundaries, and coordinates restore with transcript mutations. |
| Tree store contract | Defines versions and boundaries as part of the node's checkpoint ([Tree store](runtime.md#tree-store)). |
| Backend storage | Implements the contract in the conversation's database with SQLite transactions. |
| Backend `rpc` relay | Passes a job's storage messages to the node and generation its record names. |
| Todo command | Validates todo data and updates it by compare-and-set. |

Root and child nodes have independent versions. A Fork copies only the root's
retained command-state versions. Child histories, command-state versions, and
execution stay with the source tree, as
[Conversation Fork](conversation-fork.md#subagents) defines. Filesystem state
is outside this storage contract.

## Fork and editing

A Fork copies the retained transcript, the boundary references, and the
versions they reference. The destination's current state is the selected
assistant boundary's version. Historical versions are copied as immutable data
under the new node; later writes are independent. The source can keep
generating throughout.

Message editing selects the `before_user` version of the target message. While
the edit is prepared, the node refuses storage messages. The accepted edit
commits the rewritten history and the restored current revision in one
transaction; a failed edit leaves both unchanged. The model and tools then see
the restored todo list.

## Acceptance

Acceptance uses scripted providers and local fixtures, never a real model.

- An early and a late Fork restore different todo versions, and the two
  conversations diverge afterwards.
- Concurrent adds keep both tasks and allocate distinct task IDs.
- A write-if with a stale revision is refused, and the handler's retry commits
  on the current version.
- A background todo update racing with an assistant completion follows one
  definite ordering; a later update never leaks into an earlier boundary.
- A version failure and a boundary-write failure roll back their whole
  transactions.
- A restart after the storage commit but before the command's output keeps the
  committed state.
- Streaming checkpoint writes cannot revert a newer storage version.
- A successful edit restores todos; a rejected edit preserves them.
- Storage messages from a job of a disposed node or a replaced history cannot
  write.
- Compaction preserves the state that every retained cutoff needs.
- A Fork remains usable after the source's database is removed.
- Shell failures do not undo earlier, separately committed todo commands.
