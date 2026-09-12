# Command Storage History

Status: final implementation contract.

## Contract

`CommandStorage` contains small, structured state owned by one agent node, such
as its todo list. Each committed mutation creates an immutable version. A
conversation boundary records the version that was current at that boundary.
Fork and transcript rewrites restore that version without executing historical
commands.

The framework owns versioning for all `CommandStorage` keys. The coding harness
owns todo validation and operations; it does not implement its own history.
`todos.json` remains a logical key. Files, processes, credentials, caches and
other external effects do not become versioned conversation state.

## Example

| Ordered action | Current command state | Recorded boundary |
|---|---|---|
| Create a conversation | V0: empty | — |
| Start processing U1 | V0 | Before U1: V0 |
| Add T1 | V1: T1 pending | — |
| Finish assistant message A1 | V1 | After A1: V1 |
| Start processing U2 | V1 | Before U2: V1 |
| Complete T1 | V2: T1 done | — |
| Finish assistant message A2 | V2 | After A2: V2 |

Fork after A1 starts with T1 pending. The source keeps T1 done. Editing U2 restores
V1 before processing its replacement. Editing U1 restores V0. Updates in either
fork subsequently create versions owned by that fork.

## Immutable versions and boundary references

The initial representation stores a complete key/value map per version. Todo
state is small and changes infrequently; direct snapshots keep restore simple
and avoid replaying a mutation log. Large files and command output belong in
their existing file/blob stores, not in this map.

The backend representation uses the existing per-conversation SQLite database:

- `command_snapshots(node_id, revision, values_json)` stores immutable versions.
- The node record's `command_revision` selects its current version.
- `session_boundaries(node_id, block_id, edge, command_revision)` records a
  command-state version at `before_user`, `after_assistant` and internal
  `after_block` boundaries used for retry and resume.

Version identity is scoped to the node. V0 is the explicit empty initial version.
Revisions are allocated uniquely within a node, including after a rewind. The
current pointer can refer to an older version, so it is not inferred from the
largest revision. Reads use the selected snapshot; there is no independently
writable current copy of `todos.json` in `HostStore` or `CodingState`.

Loaded values may be cached as a derived view of the current revision. Callers
receive detached values and cannot mutate storage through a returned object.
Snapshot payloads and boundary references are validated at the store boundary.
The framework validates portable JSON and key structure; the todo command
validates the todo schema. An invalid or missing referenced version is an error,
not an empty list or a fallback to the source's latest state.

Versions remain available for the lifetime of the conversation in the initial
implementation. Fork copies the versions referenced by its retained boundaries
into the destination database. It does not depend on the source database's
continued existence.

## When a boundary is recorded

The agent records `before_user` when it starts processing a user turn, before
preparation hooks or commands for that turn mutate state. Queue admission alone
does not establish this boundary. It records `after_assistant` when the selected
text finishes. These are explicit transcript boundaries, not wall clock
comparisons and not the time when the browser receives a message.

A command can update todos while a provider is streaming or after a shell tool
returns with a background job still running. Therefore:

- Every successful command-state mutation commits immediately; it does not wait
  for `after_tool_call` or the end of a model turn.
- Assistant completion and state commits share the node's ordering. A commit
  ordered before completion is included in that message's boundary. One ordered
  after completion is excluded.
- Storage history is separate metadata. It does not insert transcript blocks
  between text deltas, split a displayed assistant message or alter provider
  replay.
- Fork eligibility uses the recorded completion boundary. It does not infer
  completion merely because another internal event was appended.

The node records equivalent internal cutoff references wherever retry, resume
or transcript rewrite requires restoration. The same selector determines the
retained transcript and its command-state boundary. Compaction does not replace
structured command state with a model-generated summary or discard versions
referenced by retained history.

## Mutation API and concurrency

The shell-level `CommandStorage` contract keeps `readJson`, `writeJson`, `delete`
and `list`, plus an atomic single-key update. Reads and mutation callbacks
receive decoded `unknown`; `undefined` means an absent key, while a stored
`null` remains a value. The command owns its value schema and validates before
using the current value. A typed callback result determines the update result:

```ts
updateJson<T>(
  key: string,
  update: (current: unknown) => T,
): Promise<T>
```

`withSignal(signal)` derives a handle with the same original history binding
and an additional invocation cancellation signal. RPC dispatch uses it for each
call so disconnect or command cancellation reaches an in-flight storage commit.

The callback is synchronous and performs no external IO. Todo add/update/done
uses this operation so two concurrent commands cannot both read the same list
and overwrite each other's changes. The framework validates and clones the
result before committing. Reads do not create versions. An unchanged value does
not create a redundant version. Deletion commits a snapshot with the key absent.

The agent node serializes state updates and message-boundary capture through a
short commit queue. It does not enqueue them behind the entire running turn.
The store commits a new snapshot and the node's current pointer in one
transaction. A transcript boundary and its version reference are also committed
together. Transcript rewrites commit retained history and the selected command
state together. All of these writes participate in the session's persistence
ordering; an older throttled checkpoint cannot overwrite a newer state commit.

Command success is returned after its storage transaction commits. Before that
point, cancellation or failure leaves the previous version current. After that
point, an output-pipe failure or cancellation does not undo the committed state;
normal command reads observe it. Fork sees either the complete old state or the
complete new state at its selected boundary, never a partially written map.

An admitted job carries a node-lifetime/history-generation token supplied
by the agent, not by the shell's environment variables. `RemoteHost` keeps
the storage handle in its local job record; the runner receives no storage token.
Nested `host shell` jobs carry the original handle. The store checks the handle's
abort signal after asynchronous media writes and immediately before its transaction. A transcript rewrite or
node disposal invalidates older invocations. A late callback from an old job
cannot commit into restored history. Pending operations reject on disposal;
the coordinator releases its queue on success, failure and cancellation. Fork
does not invalidate invocations in the source.

## Framework and backend integration

| Module | Responsibility |
|---|---|
| `shell/command.ts` | The platform-neutral storage interface, including atomic update; no knowledge of transcripts or todo fields. |
| `agent/node/` | Own the command-storage instance and authenticated invocation lifetime for each root or child. Supply that instance with the node's command registry. |
| `agent/session/` | Order state commits, capture message cutoffs, and coordinate restore with transcript mutations. |
| `agent/store/` | Define immutable snapshot and boundary contracts as part of session/tree persistence. |
| `backend/storage/` | Implement the contracts in the conversation database using SQLite transactions. |
| `backend` RPC dispatch | Use the invoking node's supplied storage instance; do not construct command storage from `execution.host.store`. |
| `coding-agent/todo-command.ts` | Validate todo data and perform atomic updates. Keep the CLI's public behavior unchanged. |

Harness state and command storage remain distinct: a harness may have its own
structured state and restoration hook, while the framework restores command
storage generically. A todo command does not need access to the whole transcript
or to mutable harness internals.

Root and child nodes have independent versions. Fork copies only the root's
retained command-state versions. Child histories, command-state versions and
execution remain with the source tree, as defined in
[Conversation Fork](conversation-fork.md#subagents). Filesystem state is outside
this storage contract.

## Fork and editing

Fork copies the retained transcript, boundary references and the snapshots they
reference. The destination's current state is the selected assistant boundary's
version. Historical versions are copied as immutable data under the new node;
future writes are independent. The source can keep generating throughout.

Message editing selects the `before_user` version for the target message. Its
preparation uses detached command state. The accepted edit atomically commits
the rewritten history and restored command-state pointer. A failed edit leaves
both unchanged. The model and tools subsequently see the restored todo list.

## Checks

- Early and late Fork restore different todo versions and subsequently diverge.
- Concurrent adds preserve both tasks and allocate distinct task IDs.
- A background todo update racing with assistant completion follows one definite
  ordering; a later update never leaks into an earlier boundary.
- Snapshot failure and boundary-write failure roll back their full transactions.
- Restart after storage commit but before command output retains committed state.
- Streaming checkpoint writes cannot revert a newer storage version.
- Successful editing restores todos; rejected editing preserves them.
- Late callbacks from a disposed node or replaced history cannot write.
- Compaction preserves the state needed by every retained cutoff.
- Fork remains usable after the source database is removed in an isolated test.
- Shell failures do not undo earlier, separately committed todo commands.

Tests use scripted providers and local fixtures, never real models.
