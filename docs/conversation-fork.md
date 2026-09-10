# Conversation Fork

Status: design under discussion; implementation has not started.

## Product behavior

Confirmed requirements:

- Fork creates a separate persistent conversation.
- Its title is the source title followed by ` (Fork)`.
- Source activity does not block Fork. A running source continues running.
- A completed assistant text message is the branch point. Streaming text becomes
  eligible after it finishes.

Proposed defaults, awaiting product confirmation:

- Retain history through the selected assistant text, including that text. Later
  messages belong only to the source.
- Open the destination after creation. It starts idle with an empty composer and
  does not call a model until the user sends a message.
- Inherit the source's current model configuration and workspace/device selection.
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
the retained transcript, restored harness state and selected model configuration;
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

These choices require product confirmation before implementation:

1. **Working directory.** Recommended behavior is to keep the same effective
   device and directory, sharing files. Workspace targets already express that
   behavior. A bare Cloud target derives its directory from the conversation ID,
   so sharing its source directory requires an explicit target representation;
   merely copying the Cloud selection produces a different directory. An
   independent filesystem snapshot or Git worktree is a separate behavior.
2. **Command storage.** Coding todos live in per-node `CommandStorage`, not in
   `CodingState` or versioned transcript snapshots. They cannot be reconstructed
   at an arbitrary historical message from the current storage value. Recommended
   initial scope is fresh command storage; copying current todos would be a
   separate, explicitly current-state copy.
3. **Subagent history.** Recommended initial scope retains child results already
   present in the root transcript, without copying the child tree. Child IDs in
   copied historical output do not identify resumable agents in the destination.

## Implementation checks

- Exact retained history and model replay for an early, middle and latest text
  cutoff, with tool results, media, compaction and harness state snapshots.
- Fork from a completed message while the source streams, runs tools or receives
  a child completion; the source keeps running and subsequent changes do not
  enter the fork.
- A source edit racing with capture has one coherent ordering.
- A destination cannot resume source jobs, children, queues or wakeups.
- Ownership, malformed requests, missing targets and conflicting UUID reuse.
- Persistence failure, restart between root commit and publication, and lost HTTP
  confirmation followed by retry.
- Product and Gallery navigation, pending/error feedback, original draft
  preservation and the exact ` (Fork)` title suffix.

Use scripted providers and local fixtures; never run tests against real models.
