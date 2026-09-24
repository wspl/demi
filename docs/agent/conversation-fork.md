# Conversation Fork

Fork copies a conversation's history, up to a completed assistant message, into
a new conversation that then continues on its own.

## Product behavior

Fork behavior:

- Fork creates a separate persistent conversation.
- Its title is the source title followed by ` (Fork)`.
- Source activity does not block Fork. A running source continues running.
- A completed assistant text message is the branch point. Streaming text
  becomes eligible after it finishes.
- Command storage keeps a general-purpose version history, including todo
  state, under the [command state history](command-state-history.md) contract.
- Fork keeps the subagent references and results already in the root
  transcript. The destination starts with no inherited subagents.

Creation defaults:

- Keep history through the selected assistant text, including that text. Later
  messages belong only to the source.
- Open the destination after creation. It starts idle with an empty composer
  and does not call a model until the user sends a message.
- Inherit the source's current model configuration, including a model switch
  the source has accepted for its next turn, and its workspace or device
  selection.
- Restore command state, including todos, at the selected message's boundary
  according to [Command state history](command-state-history.md#fork-and-editing).
- Keep the source's unsent composer draft in the source.
- Leave the destination unpinned and unarchived, with a new creation time.
- Each explicit Fork appends the suffix literally, including when the source
  title already ends in ` (Fork)`. The title's origin is `user`, so it is not
  regenerated ([Conversation titles](../product/product.md#conversation-titles)).

For example, for `U1 → A1 → U2 → A2`, Fork on `A1` creates a conversation whose
history is `U1 → A1`. If the source is generating `A2`, generation continues
there. Sending `U3` in the destination produces `U1 → A1 → U3 → A3`
independently.

## Responsibilities

| Part | Responsibility |
| --- | --- |
| Agent transcript | Selects and validates the retained prefix by a text block ID; preserves the replay structure, media, and applicable compaction data. |
| Agent session | Captures an independent seed from a live source without scheduling an action on it. |
| Agent server | Prepares a seed from a live or a stored source, and initializes a new root from it through the tree store. The destination's live runtime comes later from the ordinary node assembly. |
| Backend conversation module | Authenticates ownership; coordinates destination creation, title and target selection, retries, and durable publication. |
| Backend HTTP route | Validates the Fork request and returns the resulting conversation with the ordinary HTTP error contract ([Conversation creation and Fork](../product/web-api.md#conversation-creation-and-fork)). |
| Control database | Records the operation and publishes the destination, independently of the source's conversation database. |
| `web-ui` | Owns the footer action, the pending, error, and retry interaction, and the presentation of completed-message eligibility. |
| `web` | Calls the HTTP route, updates product state, and navigates to the created conversation. |
| `web-gallery` | Supplies fixtures and handlers for success, failure, retry, and a running source. |

Fork is an agent capability and a backend product operation. `demi agent`, the
shell command for managing subagents, gains no subcommand. Fork creates another
root, not a child of the source. Providers need no new protocol, and the
browser needs no new conversation-socket frame for this product action.

## The fork seed

The agent server has two Fork operations: prepare an owned seed from a source,
and initialize a destination root from that seed. The seed is a root
checkpoint: the retained transcript, the command state with the versions its
boundaries reference, the selected model configuration, the cwd, and the
harness name. It holds no source runtime or persistence handle.

For a live source, the session captures and copies the selected prefix in one
step, before any preparation or persistence waits. For a stored source, the
server reads one committed checkpoint through the tree store; the source must
be a root. Both paths use the same prefix preparation. Preparation never aborts
the source, waits for its whole turn to finish, or uses the source's
tree-mutation reservation. A racing history rewrite either precedes the
capture, in which case the target is validated against the rewritten history,
or follows it, in which case the captured seed remains independent.

The cutoff refers to the transcript, not the visible page. It keeps earlier
thinking, tool results, hidden inputs, and attachments. It does not replay
tools to reconstruct history or copy later state backward. The selector must
establish a valid replay boundary, including completed tool calls in the
prefix. An unavailable or unfinished target is an explicit error; the server
does not silently substitute another message or repair an invalid prefix. The
prefix ends at the selected text; response usage after it is excluded. Provider
replay uses the retained user, assistant, and tool content normally.

The command-state version is the one bound to the selected assistant message's
completion, not the source's current version. The retained history carries the
versions needed for a further Fork or an edit in the destination.

The destination starts idle, with an empty queue and a new root identity. It
has no inherited wakeup, pending steer, pending agent message, edit receipt,
child, or running shell handle. Provider execution begins through ordinary node
assembly, with a fresh runtime, when the destination is opened for use.

The session copy that compaction uses
([Session copy](compaction.md#session-copy)) is a different operation: it
copies part of a session's history in memory for one summary request and is
never stored. A Fork is a persistent root at an earlier message.

## Backend creation and retries

The endpoint is `POST /api/conversations/:sourceId/fork`, with a strict body
that contains a client-generated destination UUID and the selected text block
ID. The server obtains the transcript content itself; the browser never uploads
a replacement history. The server verifies ownership of the source and of the
destination UUID before preparing data.

The response contains the ordinary conversation summary and the complete
inherited model selection. The browser initializes the destination's composer
with that model's thinking and service-tier settings, then opens the
destination if the user is still viewing the source. `web-ui` keeps the pending
state and the retry UUID per message, above the virtualized rows; failures
appear beside that message's Fork action.

The UUID identifies one creation attempt. Repeating that attempt returns the
same destination. Reusing the UUID with a different source or cutoff is a
conflict. Requests for one destination UUID are handled one at a time. After
completion, another explicit Fork allocates another UUID. A retry after lost
confirmation must not create a second conversation or recapture newer data over
an already committed destination.

Control metadata and conversation history use different SQLite files. The
coordinator therefore needs a durable creation operation and a publication
boundary: ordinary listing and opening expose the destination only after its
root checkpoint is committed. Recovery completes the publication of a committed
root; failures do not publish an empty or partial conversation.

- `conversation_fork_operations` records creation attempts in the control
  database, reserving the destination UUID, owner, source, cutoff, and creation
  metadata ([Control records](../backend/storage.md#control-records)).
- After the destination root commits, a control transaction inserts the public
  conversation and its attached-host metadata. Publication is determined by the
  public conversation row.
- Startup recovery publishes reserved operations whose destination root
  exists; uncommitted operations stay hidden until the same request is retried.
- Ordinary conversation creation cannot use a UUID reserved for Fork.

Attachments keep their original content and timestamps. The backend can reuse
immutable blobs in the same user's blob namespace while keeping transcript rows
independent. It copies the change-store objects of the retained blocks into the
destination's namespace before the destination root is checkpointed
([The change store](../execution/edit-tracking.md#the-change-store)). Fork does
not need a model call or a device wakeup to copy conversation data.

## Execution environment and auxiliary state

The destination keeps the source's effective device and directory, sharing
files. Workspace and device targets keep their selection. A Cloud target stores
the source's resolved directory in its optional `path` field; an ordinary Cloud
target without that field uses its own conversation directory. Host resolution
respects the explicit path after a wakeup and a restart.

Attached-host names and directories are copied as configuration; a device
revoked before the destination is published is left out, as its revocation
detached it from the source. Processes, shell handles, and jobs are not copied. The destination's first ordinary
execution uses its own node identity and command storage on the shared
filesystem.

Versioned command storage is a prerequisite of this Fork design. Its snapshot,
message-boundary, and atomic restore contracts are defined in
[Command state history](command-state-history.md).

## Subagents

The destination is a new root with its own empty set of children. The retained
root transcript includes earlier subagent tool calls, IDs, and results exactly
as recorded. Fork copies no child session records, child transcripts, child
command state, running jobs, or pending completion deliveries.

For example, the source starts child S, finishes assistant message A1, and then
S continues working. Fork after A1 keeps the root's record of starting S. S
continues in the source and delivers its eventual result there. The destination
has no S in its agent tree and receives no later result from S. If S's result
was already in the root transcript before A1, that result is kept normally.

The destination's tree supervises its own children in the ordinary way
([Subagents](subagents.md)). Fork does not register source children, redirect
commands to the source, or restore source children when the destination
reopens. Newly spawned children belong to the destination.

There is no child snapshot storage, historical-child API, special historical
child interface, or Fork-specific model context. Copying the root transcript
and ordinary child ownership define the behavior.

## Acceptance

Acceptance uses scripted providers and local fixtures, never a real model.

- Exact retained history and model replay for an early, a middle, and the
  latest text cutoff, with tool results, media, compaction, and command-state
  versions.
- A Fork from a completed message while the source streams, runs tools, or
  receives a child completion: the source keeps running, and its later changes
  do not enter the Fork.
- A source edit racing with the capture has one coherent ordering.
- A destination cannot resume source jobs, children, queues, or wakeups.
- A source child continues running and delivers its result only to the source;
  the destination keeps only child references and results already in its
  prefix.
- The destination has no inherited child records or pending child completions,
  including after a restart or another Fork.
- Ownership, malformed requests, missing targets, and conflicting UUID reuse
  are refused.
- A persistence failure, a restart between the root commit and publication, and
  a lost HTTP confirmation followed by a retry each leave exactly one published
  conversation or none.
- Product and gallery navigation, pending and error feedback, preservation of
  the source's draft, and the exact ` (Fork)` title suffix.
