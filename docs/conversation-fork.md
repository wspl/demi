# Conversation Fork

Status: design under discussion; implementation has not started.

The [upstream implementation comparison](fork-implementation-comparison.md)
records source evidence and recommendations separately from this proposal.

## Product behavior

Confirmed requirements:

- Fork creates a separate persistent conversation.
- Its title is the source title followed by ` (Fork)`.
- Source activity does not block Fork. A running source continues running.
- A completed assistant text message is the branch point. Streaming text becomes
  eligible after it finishes.
- Command storage retains a general-purpose version history, including todo
  state. Detailed storage contracts remain under discussion.

Proposed defaults, awaiting product confirmation:

- Retain history through the selected assistant text, including that text. Later
  messages belong only to the source.
- Open the destination after creation. It starts idle with an empty composer and
  does not call a model until the user sends a message.
- Inherit the source's current model configuration and workspace/device selection.
- Restore command state, including todos, at the selected message's boundary
  according to [Command Storage History](command-storage-history.md).
- Preserve read-only subagent history at the selected boundary without inheriting
  child execution, following the subagent snapshot policy below.
- Keep the source's unsent composer draft in the source.
- Leave the destination unpinned and unarchived, with a new creation time.
- Each explicit Fork appends the suffix literally, including when the source
  title already ends in ` (Fork)`.

For example, for `U1 → A1 → U2 → A2`, Fork on `A1` creates a conversation whose
history is `U1 → A1`. If the source is generating `A2`, generation continues there.
Sending `U3` in the destination produces `U1 → A1 → U3 → A3` independently.

## Responsibilities

| Module | Responsibility |
|---|---|
| `agent/transcript/` | Select and validate the retained prefix using a text block ID; preserve replay structure, media and applicable compaction data. |
| `agent/session/` | Capture an independent snapshot without scheduling an action on the source; reconstruct harness state from the retained prefix. |
| `agent/server/` and `agent/node/` | Expose a server-side fork preparation/initialization API; create a new root checkpoint through `AgentTreeStore`. Live runtime assembly remains in `node/assemble.ts`. |
| `backend/conversation/fork.ts` | Authenticate ownership, coordinate destination creation, title and target selection, retries and durable publication. |
| `backend/http/conversations.ts` | Validate the Fork request and return the resulting conversation using the ordinary HTTP error contract. |
| `backend/storage/` | Persist the operation and destination independently of the source conversation database. |
| `web-ui` | Own the footer action, pending/error/retry interaction and completed-message eligibility presentation. |
| `web/conversation/` and `web/api/` | Call HTTP, update product state and navigate to the created conversation. |
| `web-gallery` | Supply fixtures and handlers for success, failure, retry and a running source. |

This is an agent framework capability and a backend product operation.
`demi agent`, the shell command for managing subagents, does not gain a new
subcommand. Fork creates another root, not a child of the source. Providers do
not need a new protocol, and the browser does not need a new agent WebSocket
command for this product action.

## Framework snapshot

The proposed server API has two responsibilities: prepare an owned fork seed
from a source and initialize a destination root from that seed. The seed contains
the retained transcript, restored harness state, command-state snapshots and
boundary references, and selected model configuration;
it contains no source runtime or persistence handle. Final API names are an
implementation detail.

For a live source, capture and deep-copy the selected prefix synchronously before
awaiting preparation or persistence. For a cold source, read one committed
checkpoint through `AgentTreeStore`. Both paths use the same prefix preparation.
Preparation never aborts the source, waits for its whole turn to finish, or uses
the source's tree-mutation reservation. A racing history rewrite either precedes
the capture, in which case the target is validated against the rewritten history,
or follows it, in which case the captured seed remains independent.

The cutoff refers to the transcript, not the visible DOM. It retains earlier
thinking, tool results, hidden inputs, attachments and applicable state snapshots.
It does not replay tools to reconstruct history or copy later state backward.
The selector must establish a valid replay boundary, including completed tool
calls in the prefix. An unavailable or unfinished target produces an explicit
error; the server must not silently substitute another message or repair an
invalid prefix. Handling response metadata adjoining the selected text belongs
to this selector and must be covered by replay tests.

The command-state version is the one bound to the selected assistant message's
completion, not the source's current version. Retained history carries the
versions needed for further Fork or editing in the destination. The framework
restores this storage independently of the harness's state restoration hook.

The destination starts with `phase: idle`, an empty queue and a new root identity.
It has no inherited wakeup, pending steer, edit receipt, child lifecycle or running
shell handle. Harness state comes from the retained prefix via the harness's
state restoration contract. Provider execution begins through ordinary node
assembly with a fresh runtime when the destination is opened for use.

`AgentSession.clone()` remains the isolated in-memory clone API. Its defaults copy
the current complete transcript and current state, and do not inherit the store.
Those defaults are not the definition of a persistent fork at an earlier message.

## Backend creation and retries

The proposed endpoint is `POST /api/conversations/:sourceId/fork`, with a strict
body containing a client-generated destination UUID and the selected text block
ID. The server obtains transcript content itself; the browser never uploads a
replacement history. The server verifies source ownership and destination UUID
ownership before preparing data.

The UUID identifies one creation attempt. Repeating that attempt returns the same
destination. Reusing the UUID with a different source or cutoff is a conflict.
After completion, another explicit Fork allocates another UUID. A retry after
lost confirmation must not create a second conversation or recapture newer data
over an already committed destination.

Control metadata and conversation history use different SQLite files. The
coordinator therefore needs a durable creation operation and a publication
boundary: ordinary listing/opening exposes the destination only after its root
checkpoint is committed. Recovery completes publication of a committed root;
failures do not publish an empty or partial conversation. The concrete storage
record and transaction sequence must be finalized before implementation.

Attachments retain their original content and timestamps. The backend can reuse
immutable blobs in the same user's blob namespace while keeping transcript rows
independent. Fork does not require a model call or a device wakeup merely to copy
conversation data.

## Execution environment and auxiliary state

The working directory requires product confirmation before implementation.
Recommended behavior is to keep the same effective device and directory,
sharing files. Workspace targets already express that behavior. A bare Cloud
target derives its directory from the conversation ID, so sharing its source
directory requires an explicit target representation; merely copying the Cloud
selection produces a different directory. An independent filesystem snapshot
or Git worktree is a separate behavior.

Versioned command storage is a prerequisite of this Fork design. Its snapshot,
message-boundary and atomic restore contracts are defined in
[Command Storage History](command-storage-history.md).

## Subagent snapshots

Proposed policy, awaiting product confirmation: preserve historical child data
while keeping every running child owned exclusively by its source tree.

For example, the source starts child S, finishes assistant message A1, and then
S continues working. Fork after A1 has the following behavior:

- S continues in the source. Its commands, files and completion delivery follow
  the source's existing execution policy.
- The destination has a read-only record of S at the A1 boundary. It displays
  `Running at fork boundary · execution not inherited`.
- S is not counted as an active child in the destination. Its history does not
  stream, no spinner remains active, and there is no pending completion to wait
  for in that conversation.
- S's later messages and result reach only the source, even if S finishes before
  the user eventually clicks Fork on A1.
- Fork does not abort S or automatically start another execution of S.

The snapshot rules apply recursively to descendants:

| State at the selected boundary | Destination history | Destination execution |
|---|---|---|
| Child not yet created | No child record | None |
| Child running, waiting or yielding | Frozen partial history and the recorded state at the boundary | None |
| Child closed, result already in retained root history | Frozen child history; retain the existing root result once | None |
| Child closed, completion not yet in retained root history | Frozen child history; do not synthesize a new root completion | None |
| Child resumed after the boundary | Preserve its earlier recorded state and history | None |

The records use a distinct historical snapshot contract. They are not inserted
as owned `AgentNodeRecord` children, are not returned by the live tree's
`AgentTreeStore.children()`, and are not registered with `ChildSupervisor` or
`AgentDirectory`. They have no delivery queue or resumable lifecycle. Copying
owned child rows is not an implementation of this policy: ordinary supervisor
restore would resume open children or deliver pending closed-child results.

A snapshot records source identity, parent relationship, description/profile,
phase at the boundary, frozen transcript and command-state reference. A source
ID is provenance, not authority to send, steer, abort or resume that agent from
the destination. A snapshot's phase can say the source was running; it must not
be represented as a newly aborted or completed destination child. Tool output
already in history retains its original IDs and text.

The first inference in the destination receives durable framework context that
historical children and shell handles are not attached to this conversation.
This context does not alter the copied transcript prefix or cause inference on
creation. An attempt to control a historical child must not be redirected to
the source. New work uses a newly created child with a new ID and an explicit
task brief; automatic restart or continuation of a historical child is outside
this policy.

### Capturing the historical tree

The selected boundary must identify immutable child data. The backend cannot
read the child's current transcript at Fork-click time and call it historical.
At each eligible assistant completion, the framework captures a consistent
read-only tree snapshot alongside the root's command-state boundary. Node
membership, lifecycle state, partial transcript content and command-state
references come from the same ordered capture. Live jobs are not stopped.
The shared `session_boundaries` record binds both command-state and subagent
snapshot references to that one cutoff; they are not captured independently and
matched afterward by timestamp.

The boundary stores references to immutable node snapshots. Unchanged node
snapshots can be reused by later boundaries; mutable child rows or transcript
revision numbers without preserved version content are insufficient. Nested
children created after the boundary are absent. Child completion or resume
racing with capture has one defined ordering, including whether the completion
has reached the retained root history.

Fork materializes these historical records under the destination's ownership.
They remain readable independently of the source conversation's lifetime. A
further Fork preserves inherited historical records along with snapshots of
children owned by that intermediate conversation at its selected boundary.

`web-ui` presents snapshots as history with read-only controls, separately from
owned live agents. The backend exposes the frozen data using a validated history
contract; the product and Gallery supply records to the same presentation.

Shared files remain a separate consideration. When both conversations use one
directory, S can continue changing those files in the source; an independent
conversation history does not freeze the filesystem.

## Implementation checks

- Exact retained history and model replay for an early, middle and latest text
  cutoff, with tool results, media, compaction, harness state snapshots and
  command-state versions.
- Fork from a completed message while the source streams, runs tools or receives
  a child completion; the source keeps running and subsequent changes do not
  enter the fork.
- A source edit racing with capture has one coherent ordering.
- A destination cannot resume source jobs, children, queues or wakeups.
- A child started before the cutoff and closed afterward stays frozen at the
  cutoff in the destination; only the source receives its eventual completion.
- Completed-but-undelivered children do not wake the destination after restore.
- Nested child creation and child resume do not leak later history into a fork.
- Historical children stay read-only after destination restart and a second Fork.
- Ownership, malformed requests, missing targets and conflicting UUID reuse.
- Persistence failure, restart between root commit and publication, and lost HTTP
  confirmation followed by retry.
- Product and Gallery navigation, pending/error feedback, original draft
  preservation and the exact ` (Fork)` title suffix.

Use scripted providers and local fixtures; never run tests against real models.
