# Message editing and resend

Editing a user message replaces that message and everything after it with a new
user turn. For example, in a conversation that reads A, answer A, B, answer B,
C, answer C, editing B removes B and everything after it and continues from B′:

```text
A → answer A → B → answer B → C → answer C
                    edit B and submit
A → answer A → B′ → answer B′
```

## Behavior and ownership

The agent session owns the operation. The request names the target block,
supplies the complete replacement content, carries a stable operation ID, and
identifies the transcript snapshot the editor showed: the transcript's version,
its epoch and revision ([Patches and versions](runtime.md#patches-and-versions)).

The transcript owns suffix replacement and the patch that replicates it. The
session prepares the replacement turn, commits the checkpoint, publishes the
accepted history, and starts inference with a fresh provider runtime, a runtime
fork ([Runtime forks and closing](../providers/providers.md#runtime-forks-and-closing)),
so that no state the provider kept about the removed history reaches the
replacement. The replacement uses a new turn ID, the selected model, and a
freshly prepared preamble. Retained blocks keep their IDs, timestamps, content,
and media.

A `user` block keeps its content as submitted. The editor opens that content,
and replay sends it.

### Editable blocks

Only a `user` block is an editable target. An explicit user submission has its
own block type, so the session and the page read editability from the block's
type, and no second rule decides it ([Transcript](runtime.md#transcript)). The
hidden inputs (`context`, `wakeup`, and `agent_message` blocks), human `steer`
blocks, and every block the model produced are not editable.

### Files the edit keeps

The replacement content refers to the files of the edited message that it
keeps and never carries their bytes. For example, a message sent with
`notes.txt` and `chart.png` holds an attachment record for each file and, for
the image the model reads natively, a media block. An edit that keeps both
files and changes only the text sends the new text, an
`{ type: "attachment", path }` reference for each record, naming the path the
record holds, and the image's `media` by blob reference. New files arrive as
uploads or remote files, as in a send
([Client frames](runtime.md#client-frames)).

The session replaces each `attachment` reference with the target message's
record for that path. A path that no attachment record of the target message
holds rejects the edit without mutation, so an edit cannot take over a file
that another message received.

### Admission

The agent server coordinates admission with the node's children. Editing
requires no active action, queued message, pending steer, pending agent
message, scheduled wakeup, live child, child start or close in progress, or
child completion awaiting delivery. Admission excludes competing actions and
completion delivery while the edit is prepared and committed, and the node's
command storage refuses storage messages during that window
([Command state history](command-state-history.md#fork-and-editing)). The
reservation ends when the edit is accepted or rejected: the replacement's turn
may start and close children again.

### Commit and idempotency

The conversation's store commits the rewritten transcript rows, the checkpoint
state with the operation's receipt, and the restored command state in one
transaction. Preparation and persistence failures preserve the accepted
history. A successful commit accepts the replacement even if inference then
fails. A disconnect does not cancel an accepted edit.

The session restores command storage to the version recorded before the target
`user` block, in the same commit as the rewritten transcript
([Command state history](command-state-history.md)). Files, completed child
records, and the external effects of tools that already ran stay outside this
restore.

The `edit_result` of an accepted edit follows the rewrite's `replace` patch
and comes before any frame of the replacement's turn.

Each accepted operation leaves a receipt in the checkpoint: its operation ID,
the replacement's turn ID, and a digest of the request. The digest is the
SHA-256 of the request's RFC 8785 canonical JSON, so equal requests have equal
digests whatever their key order
([Encodings and digests](../backend/storage.md#encodings-and-digests)). The
digest covers the request as the browser sent it, before any upload in it is
resolved. Resolving an upload writes the file under a name not yet taken, so
the same request resolved twice would differ, while its retry must match.

A request is checked in this order, before the backend resolves its uploads,
so a repeated request writes no file:

1. If its operation ID has a receipt, the request returns that receipt without
   applying the rewrite again when the digests match, and is rejected as a
   conflict when they differ.
2. If an edit with its operation ID is still being prepared, the request shares
   that edit's acceptance when the digests match, and is rejected when they
   differ.
3. Otherwise the snapshot is checked. An edit whose snapshot is stale is
   rejected without mutation, including after a server restart or a later
   edit, so a replayed request cannot restore a superseded transcript suffix.
   A snapshot taken before a restart is always stale, because every restore
   starts a new epoch.

### The editor

`web-ui` owns the editor and the submission interaction; `web` supplies product
state and handlers, and `web-gallery` supplies examples of the same components.
The page offers editing only on the last `user` block; blocks of other types
after it (assistant output, hidden inputs, agent messages, and steers) do not
move that target. This is a presentation rule: the session accepts an edit of
any `user` block.

The edit composer opens the original content as it was sent: its formatted
text, with each attachment a capsule in its place
([Writing a message](../product/product.md#writing-a-message)). The target
message and the transcript blocks after it stay visible at reduced opacity and
cannot receive pointer or keyboard interaction. The × to the left of the send
button, or Escape, leaves an unsubmitted edit and restores the unrelated
composer draft. Text and capsules edit as one text, and the submitted content
keeps them in the order shown. Deleting a capsule drops its attachment. The
edit composer adds files the way the main composer does
([Attachments](../product/product.md#attachments)): an added file is uploaded
and becomes a capsule where it is dropped or pasted, and an upload still
running stops when the edit is left. The edit request carries no file bytes: a
new file travels as its upload, and native media the message already holds,
such as an image, a video, or a PDF, travels as `media` by blob reference
([Client frames](runtime.md#client-frames)). Keys are those of the composer:
Enter submits, Shift+Enter inserts a newline, and an active IME composition
does not submit.

Entering or leaving editing does not modify the transcript. Save and resend
submits one operation. The draft and its attachments stay recoverable until the
client confirms acceptance. An uncertain result is reconciled by operation ID
before resubmission. The sending and uncertain phases lock the content and the
exit controls; an explicit rejection returns to editing. Successful acceptance
restores the composer draft.

## Acceptance invariants

Acceptance observes four independently obtained results: the session
transcript, the client transcript reconstructed from frames, the checkpoint
loaded from storage, and the requests delivered to the provider. Provider
assertions use hand-authored expected inference items; building the expected
result with the production replay would conceal replay defects.

1. The retained prefix is unchanged. The replacement appears once. No removed
   answer, steer, tool result, or invalid summary reaches later inference.
2. Before durable acceptance, neither clients nor the replacement provider
   observe the candidate history. A failed save leaves both memory and storage
   unchanged, including after pending checkpoint work has finished.
3. After acceptance, reconnect and restart recover the same replacement
   history. A generation error does not restore the removed suffix or
   duplicate the input.
4. Each accepted operation performs at most one rewrite. This is not a
   guarantee of one provider request: tool continuations, compaction, and
   explicit recovery can make more requests under their own contracts.
5. Rejected, failed, and cancelled preparation releases reservations and
   candidate resources. A successful replacement releases the discarded
   provider runtime. Completions, timers, and pending writes of a discarded
   action cannot change the replacement.

## Fixtures and deterministic scheduling

- A three-turn transcript contains distinct sentinel values for each user
  message, assistant answer, steer, and tool call and result. Attachments have
  distinct bytes, including two files with the same name.
- Session cases use an in-memory session store and scripted providers. The
  recording provider gives each runtime its own identity, captures detached
  requests, and records each close. Runtimes share only the script and the
  observation sink; each runtime fork owns its own consumed context and
  continuation state.
- The Claude Code provider is observed through a fake CLI transport that
  records transport starts, input writes, and termination. No installed CLI or
  model account is required.
- Barriers hold the operation at preparation, save, publication, and provider
  start, and both orders of competing operations are exercised explicitly.
  Wakeups run on a controlled clock; arbitrary sleeps are not synchronization.
- Database cases use a temporary on-disk conversation database and blob store,
  and inject a failure inside a real transaction after some statements have
  run. A fake store that fails before any write does not establish
  transactional rollback.
- Every case releases its barriers and disposes its sessions, providers,
  transports, timers, and temporary storage at teardown, including after a
  failed assertion.

## Transcript and compaction cases

| Case | Required observation |
| --- | --- |
| Edit the first, a middle, or the last user turn | The exact retained prefix and one replacement; the suffix's IDs disappear. |
| Several text blocks and attachments | All replacement blocks arrive in order; preserved bytes and metadata are equal. |
| The suffix contains steers, tool results, abort and resume blocks, and hidden inputs | None survives the cut or leaks into replay. Retained tool calls and results stay correctly paired. |
| Edit before a compaction boundary | The summary covering the edited content disappears; the retained original blocks supply the context. |
| Edit after a compaction boundary | The valid prefix summary remains; the replaced turn and later content disappear. |
| Several boundaries, or a boundary inside a turn | Only summaries describing retained content can reach inference. Boundary and marker references stay valid under replay and context accounting. |
| The cut removes a compaction marker or the latest usage response | Context estimation does not use an invalid usage anchor ([Context estimate](compaction.md#context-estimate)). A preflight summary request contains only retained content, and the replacement where applicable. |
| Patch application and reload | Applying the emitted patches to the client's initial copy produces the accepted transcript exactly; save and load produce the same blocks. |

The edit keeps every block before the target and nothing after it, so the
first, middle, and last positions cover the cut when the history holds each
kind of block the cut removes: a tool call with its result, a steer, and plain
turns. The prefix and replacement oracle, the blocks before the target compared
with the history before the edit, is separate from the semantic assertions on
inference items and on patch sequences.

## Session, provider and admission cases

| Case | Required observation |
| --- | --- |
| Edit B in the three-turn fixture | The generation request contains A, answer A, and B′; sentinel values of B, answer B, C, and answer C are absent. |
| Edit the final user message when it is not the first | A fresh runtime receives the replacement, although the user-message count and the first user message are unchanged. |
| Replace an attachment with a file of equal name and type but different bytes | A fresh runtime receives the new bytes; equal text, file names, and media types do not permit stale context reuse. |
| A stateful provider has consumed the removed content | No continuation is sent to its transport; replacement inference starts independently. Disposing the discarded runtime does not end the replacement runtime. |
| Preamble preparation fails | No published rewrite, durable mutation, or replacement inference; the next valid operation can acquire admission. |
| Preparation is aborted | The candidate is discarded, the accepted transcript gains no candidate abort or output blocks, and acquired resources are released. |
| An abort arrives while the commit is in progress | The durable outcome decides: a failure preserves the accepted history; a success keeps the replacement and does not start generation once the cancellation has been accepted. |
| The model fails before output or after a completed tool | The replacement stays accepted. Explicit recovery follows the resume contract and does not repeat a completed tool because submission was retried. |
| An active action, queued message, pending steer, pending agent message, wakeup, live child, or undelivered completion | Editing is rejected without deleting, consuming, or silently cancelling that work. |
| A send, a model or target change, a child resume, or a completion delivery races with the edit | Exactly one admissible ordering takes effect; the loser observes busy or conflict, or operates on the committed state. Nothing enters the preparation window. |
| The target is a `steer`, `context`, `wakeup`, or `agent_message` block, or a block the model produced | The edit is rejected without mutation. |
| Completed external effects | A file written in the removed suffix and an archived child record remain; command state returns to the version before the edited message; delivered child completions are not replayed because their blocks in the parent were removed. |

## Durability and failure boundaries

These cases run against the backend's real conversation database and blob
store, and against an authenticated backend with a scripted model across a
restart.

| Fault or pause | Required observation |
| --- | --- |
| The save is blocked | Connected clients keep the accepted transcript; replacement inference has not started. |
| Blob externalization fails | No checkpoint changes; retained media stays readable. |
| The database fails after block writes or deletions and before the transaction completes | Reopening the database shows the complete pre-edit checkpoint, never mixed blocks and state. |
| An earlier checkpoint write is in flight | The edit commit is ordered after it; no delayed write restores removed rows or overwrites the replacement state. |
| The commit succeeds but the acceptance frame is lost | Reconnect recovers the replacement; repeating the operation produces no second rewrite. |
| The process exits before the commit | Reload recovers the complete accepted pre-edit checkpoint. |
| The process exits after the commit and before publication or provider start | Reload recovers the replacement and an actionable unfinished turn. Retrying the submission confirms acceptance; inference recovery is a separate action. |
| The process exits after generated output has been checkpointed | Reload keeps that output and uses ordinary recovery of an incomplete turn where needed. |

A crash boundary is observed by ending a separate process that runs a scripted
session over the backend's real database and blob store, without disposal or a
final checkpoint, and reopening that database in a fresh process. A graceful
restart of the full authenticated backend is a different failure boundary and
is observed separately.

## Protocol and UI cases

- Malformed content, invalid targets, unauthorized conversation access, and
  stale snapshots are rejected without provider calls or transcript changes.
- Repeated operation IDs are exercised before acceptance, after acceptance,
  after reconnect and restart, and after a later edit removes the replacement
  turn. Identical retries do not rewrite again; conflicting payloads are
  rejected.
- Two editors open from the same snapshot. One edit is accepted; the other,
  submitted through a connection that took the conversation over, is rejected
  as a conflict.
- A dropped transcript patch requires snapshot resynchronization, and a lost
  acceptance frame requires reconciliation. Neither case guesses success from
  message text or silently generates a fresh operation ID.
- Cancel preserves the transcript and the unrelated composer draft. Save
  preserves every text block and attachment, blocks duplicate activation while
  pending, and keeps the edited draft after a validation or persistence
  failure.
- After acceptance, a generation failure shows turn recovery instead of
  offering to submit the accepted edit as another new message.
- Gallery examples cover editing, saving, conflict, and failure with the shared
  `web-ui` behavior. Browser acceptance covers keyboard submission, Chinese
  input, multiline text, attachments, cancel, reconnect, and the replaced
  transcript. The shared keyboard predicate is tested with active IME
  composition events. The product acceptance fixture supplies a scripted
  backend and checks the captured requests as well as the visible page.

## Completion gate

Editing is accepted when every invariant group above has automated coverage
and the shared gallery and product browser acceptance passes. Screenshots
alone do not establish transcript correctness or durability. No case calls a
real model or generates a live compaction fixture.

The strength of the suite is checked with temporary targeted defects: retain
one removed block, reuse the consumed provider runtime, publish before save, or
accept a stale snapshot. Each defect must fail its corresponding assertion, and
each is removed again before the checkpoint.
