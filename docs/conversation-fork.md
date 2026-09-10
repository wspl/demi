# Conversation Fork

Status: final implementation contract.

The [upstream implementation comparison](fork-implementation-comparison.md)
records source evidence and recommendations separately from this design.

## Product behavior

Fork behavior:

- Fork creates a separate persistent conversation.
- Its title is the source title followed by ` (Fork)`.
- Source activity does not block Fork. A running source continues running.
- A completed assistant text message is the branch point. Streaming text becomes
  eligible after it finishes.
- Command storage retains a general-purpose version history, including todo
  state, under the command-storage history contract.
- Fork retains subagent references and results already in the root transcript.
  The destination starts with no inherited subagents.

Creation defaults:

- Retain history through the selected assistant text, including that text. Later
  messages belong only to the source.
- Open the destination after creation. It starts idle with an empty composer and
  does not call a model until the user sends a message.
- Inherit the source's current model configuration and workspace/device selection.
- Restore command state, including todos, at the selected message's boundary
  according to [Command Storage History](command-storage-history.md).
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

The server API has two responsibilities: prepare an owned fork seed
from a source and initialize a destination root from that seed. The seed contains
the retained transcript, restored harness state, command-state snapshots and
boundary references, and selected model configuration;
it contains no source runtime or persistence handle. `AgentServer.prepareFork` captures the seed; `initializeFork` persists its new root.

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
invalid prefix. The prefix ends at the selected text; response usage metadata after it is excluded.
Provider replay uses the retained user, assistant and tool content normally.

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

The endpoint is `POST /api/conversations/:sourceId/fork`, with a strict
body containing a client-generated destination UUID and the selected text block
ID. The server obtains transcript content itself; the browser never uploads a
replacement history. The server verifies source ownership and destination UUID
ownership before preparing data.

The response contains the ordinary conversation summary and the complete inherited
model selection. `web/conversation/store.ts` initializes the destination composer
with that model's thinking and service-tier settings, then `ChatPage.vue` opens
the destination if the user is still viewing the source. `web-ui/agent/message-fork.ts`
keeps the pending state and retry UUID per message above virtualized rows; failures
appear beside that message's Fork action.

The UUID identifies one creation attempt. Repeating that attempt returns the same
destination. Reusing the UUID with a different source or cutoff is a conflict.
After completion, another explicit Fork allocates another UUID. A retry after
lost confirmation must not create a second conversation or recapture newer data
over an already committed destination.

Control metadata and conversation history use different SQLite files. The
coordinator therefore needs a durable creation operation and a publication
boundary: ordinary listing/opening exposes the destination only after its root
checkpoint is committed. Recovery completes publication of a committed root;
failures do not publish an empty or partial conversation. `conversation_forks` in the control database reserves the destination UUID, owner,
source, cutoff and creation metadata. After the destination root commits, a control
transaction inserts the public conversation and its attached-host metadata.
Publication is determined by the public conversation row. Startup recovery publishes
reserved operations whose destination root exists; uncommitted operations remain
hidden until the same request is retried. Ordinary conversation creation cannot use
a UUID reserved for Fork.

Attachments retain their original content and timestamps. The backend can reuse
immutable blobs in the same user's blob namespace while keeping transcript rows
independent. Fork does not require a model call or a device wakeup merely to copy
conversation data.

## Execution environment and auxiliary state

The destination keeps the source's effective device and directory, sharing files.
Workspace and device targets retain their selection. A Cloud target stores the
source's resolved directory in its optional `path` field; an ordinary Cloud target
without that field uses its own conversation directory. Host resolution respects
the explicit path after wakeup and restart.

Attached-host names and directories are copied as configuration. Processes,
shell handles and jobs are not copied. The destination's first ordinary execution
uses its own node identity and command storage on the shared filesystem.

Versioned command storage is a prerequisite of this Fork design. Its snapshot,
message-boundary and atomic restore contracts are defined in
[Command Storage History](command-storage-history.md).

## Subagents

The destination is a new root with its own empty child registry. The retained
root transcript includes earlier subagent tool calls, IDs and results exactly as
recorded. Fork copies no child session records, child transcripts, child command
state, running jobs or pending completion deliveries.

For example, the source starts child S, finishes assistant message A1, and then
S continues working. Fork after A1 retains the root's record of starting S.
S continues in the source and delivers its eventual result there. The destination
has no S in its agent tree and receives no later result from S. If S's result was
already in the root transcript before A1, that result is retained normally.

The existing root-scoped `AgentDirectory` and `ChildSupervisor` operate on the
destination's own children. Fork does not register source children, redirect
commands to the source, or restore source children when the destination reopens.
Newly spawned children belong to the destination in the ordinary way.

There is no child snapshot storage, historical-child API, special historical
child interface, or additional Fork-specific model context. Root transcript
copying and ordinary child ownership define the behavior.

## Implementation checks

- Exact retained history and model replay for an early, middle and latest text
  cutoff, with tool results, media, compaction, harness state snapshots and
  command-state versions.
- Fork from a completed message while the source streams, runs tools or receives
  a child completion; the source keeps running and subsequent changes do not
  enter the fork.
- A source edit racing with capture has one coherent ordering.
- A destination cannot resume source jobs, children, queues or wakeups.
- A source child continues running and delivers its result only to the source;
  the destination retains only child references/results already in its prefix.
- The destination has no inherited child records or pending child completions,
  including after restart or another Fork.
- Ownership, malformed requests, missing targets and conflicting UUID reuse.
- Persistence failure, restart between root commit and publication, and lost HTTP
  confirmation followed by retry.
- Product and Gallery navigation, pending/error feedback, original draft
  preservation and the exact ` (Fork)` title suffix.

Use scripted providers and local fixtures; never run tests against real models.
