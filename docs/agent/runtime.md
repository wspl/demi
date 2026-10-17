# Agent runtime

The agent runtime runs the agents of every conversation. Each agent is a
session: it keeps a transcript, asks a provider for the next response, runs the
tools the model requests, and saves its checkpoint in the conversation's
database. The web app follows and controls a conversation over one WebSocket
that carries agent frames.

For example, a user sends "run the tests and fix what fails":

```text
web app (ConversationClient)        backend: the user's shard                   Host
---------------------        -------------------------                   ----
send ----------------------> conversation socket
                               -> tree -> root session
                                    user block
                                    provider request
                                    text + shell call
                                    save, run the tool ----------------> runner job
                                    tool result <----------------------
                                    provider request
                                    text + response: the turn ends
transcript_patch <---------- one outbox per connection
                               tree store -> conversation database
```

The root session appends the message as a `user` block and asks the provider
for a response. The model answers with some text and a `shell` call. The
session saves the call as executing, runs the command on the conversation's
Host, records the result, and asks the provider again. When a response
requests no tool, the turn ends. Each change reaches the web app as a
transcript patch and the conversation's database as changed rows.

This document owns sessions and turns, their input, the identifiers the
model sees, and the tree store contract. Related rules have their own homes:

- The session tree, `demi agent` and agent messages:
  [Subagents](subagents.md).
- Compaction, token estimates and truncation:
  [Compaction](compaction.md).
- Provider failures, retries and recovering an unfinished turn:
  [Failures and recovery](failures-and-recovery.md).
- [Message editing](message-editing.md) and
  [Conversation Fork](conversation-fork.md).
- The thread a session runs on: [the user's shard](../architecture/concurrency.md#the-user-shard).
- The crates that implement it: [Crates](../architecture/crates-and-packages.md#crates).
- The `shell` tool, its results and command reports: [The shell tool](shell-tool.md).
- The transcript, its blocks, replay and versions: [Transcript](transcript.md).
- What the page draws and the frame protocol: [Rendering](rendering.md).

## Sessions and turns

A session belongs to one agent node: the root of a conversation or one of its
subagents. Every node has the same kind of session, built by the same assembly
([Runtime](subagents.md#runtime)). A session holds its transcript and command
state, its model selection with its own provider runtime, the actions waiting
to run, the pending steers, the command reports not yet written, and one status.
Sessions run on the user's shard thread, where no other work runs while a
session changes its state.

The agent runtime knows no product and no plugin. The product gives the
agent server what every node is assembled from, and answers three questions
while a node runs. The command set may change while the server runs: a tree
takes it when it opens, records its revision, and keeps it until it closes,
so a change reaches a conversation when its tree opens again
([A user's plugins](../architecture/plugins.md#a-users-plugins)). The
profiles and the Subagent switch are asked at each spawn instead, so a change
reaches the next spawn ([Profiles](subagents.md#profiles)).

| The product supplies | What it is | Where it comes from in Demi |
| --- | --- | --- |
| The command set | The commands every node starts from; the runtime adds its own groups per node ([Tools](shell-tool.md#tools)) | The commands of the plugins the user has on, and the `demi host` and `demi attachment` groups ([Plugins](../architecture/plugins.md#commands)) |
| Instructions | The identity that opens the system prompt ([System prompt](system-prompt.md)) | The product's instructions; a plugin adds only its command groups' index entries ([Prompt text and context](../architecture/plugins.md#prompt-text-and-context)) |
| Subagent settings | Whether subagents are on, their [limits](subagents.md#limits), and the named [subagent profiles](subagents.md#profiles) with whether each is enabled, as data, asked at each spawn and each `demi agent profiles` | The user's current settings, which the user edits in the Subagents section of settings |
| Provider runtimes | A node's model selection and the runtime that serves it: the root's from the conversation's record, a child's from its profile's model settings or its parent's ([Runtime](subagents.md#runtime)) | The backend's provider assembly ([Inference admission and runtime ownership](../providers/providers.md#inference-admission-and-runtime-ownership)) |
| The Host of a node | Where its shell tools run now, asked at each shell tool call | The conversation's host access ([Host operations](../execution/sessions-and-targets.md#host-operations)) |
| Context sources | What the model must learn before a request, asked before each one ([Context](transcript.md#context)) | The conversation's execution context, then each plugin that is a context source, while the user has it on |

A node's system prompt is therefore its identity (the instructions, or a
profile's, which replace them), the harness guide, the runtime's rules for
its tool, the capability index of the node's commands and the model
identity, in that order ([System prompt](system-prompt.md)). It is rendered once, when the node is
assembled, and holds no time, id, Host or state
([Prompt cache](../providers/providers.md#prompt-cache)).

### Actions

Work reaches a session as actions. One action runs at a time, and the others
wait in arrival order.

| Action | Comes from | What it does |
| --- | --- | --- |
| Send | A `send` frame, or the next queued message | Appends a `user` block and runs a turn |
| Continuation | A command report or an agent message while nothing runs | Appends a `wakeup` block, or the waiting agent messages, and runs a turn; never shown in the queue |
| Retry | A `retry` frame | Rewinds the last input turn and runs it again ([Recovery](failures-and-recovery.md#recovery-is-one-mechanism)) |
| Resume | A `resume` frame | Unwinds the unfinished turn to its resume point and continues it ([Recovery](failures-and-recovery.md#recovery-is-one-mechanism)) |
| Compact | A `compact` frame | Runs one compaction pass ([Compaction](compaction.md#compaction)) |
| Edit and send | An `edit_and_send` frame | Replaces a user message and everything after it, then runs a turn ([Message editing](message-editing.md)) |

Admission is decided when an action arrives:

- A disposed session refuses every action.
- While an edit is being prepared, the session refuses sends, the other
  actions, steers, agent messages and command reports. A model switch waits
  for the edit instead ([Model switch](#model-switch)).
- `retry`, `resume` and `compact` are refused unless the session is idle, with
  the reason `Session is busy (<phase>)`. A `compact` is also refused below
  half the window in use
  ([When compaction runs](compaction.md#when-compaction-runs)).
- A send is never refused because the session is busy: it waits in the queue
  ([Input](#input)).

A running action holds a lease on its tree's admission. A target switch or an
archive reserves the idle tree through the same admission
([Switch the primary target](../execution/sessions-and-targets.md#switch-the-primary-target)),
and a running turn is conversation activity
([Activity](../execution/resource-lifecycle.md#activity)).

### A turn

A send, a continuation, a retry, a resume and an accepted edit each run a turn:

```text
action starts
  prepare the input: a new block, or the rewound or unwound history
  land a recorded model switch, compacting for the new model first
  compact first when the history is over the threshold
  |
  +-> write pending steers and agent messages into the transcript
  |   request the provider, retrying transient failures
  |   refused for its tokens: compact once and request again
  |   refused for its size: the turn fails
  |   apply the provider's events to the transcript as they stream
  |   save, then run the requested tools one at a time, in order
  |   usage reached the compaction threshold: compact, append `resume`, loop
  |   steers or agent messages arrived during this round: loop
  +-- tools ran and none asked to end the turn: loop
  |
action ends
  discard human steers still pending, save,
  start the next waiting action
```

A turn ends when a response requests no tool. Input that arrived during the
round overrides that: the turn asks the provider once more. Transient provider
failures are retried inside the request
([Retries](failures-and-recovery.md#retries)); compaction inside a turn,
after a response over the threshold and after a request refused for its
tokens, is described in [Compaction](compaction.md#when-compaction-runs).

While an action runs, its stage is one of: preparing (before the first
request), streaming from the provider, running tools, compacting, or
finalizing (saving at the end). Clients see the phase `idle`, `running` or
`compacting`: `compacting` while a pass runs, and `running` in every other
stage, finalizing included. The phase turns `idle` only once the save that
ends the action has committed and no other action starts; the next waiting
action starts without an `idle` between the two. For example, the user queues
a message while the agent answers: the page sees `running` through the answer,
its closing save and the queued message's turn, and `idle` once that turn's
save has committed. So a client that sees `idle` can edit at once
([Admission](message-editing.md#admission)), and what it then reads from the
backend includes the finished turn. The phase, the visible queue and whether
the session has settled are derived from the one status and the waiting
actions; no second flag records them.

The session reports each change as plain data once the change is complete, in
the order the changes happened. A listener may call back into the session;
what that causes is reported after the current event has reached every
listener. The conversation's connection turns these events into frames
([Frame protocol](rendering.md#frame-protocol)).

### Stop

The user's Stop sends `abort`. Each `abort` stops one thing, in this order:

1. A running action. The session stops it and waits until the action has
   recorded the stop, then replies with what was running: the provider
   stream, a tool, a compaction, or the turn.
2. Otherwise, the first waiting action. A queued message leaves the queue; any
   other action is dropped.
3. Otherwise, nothing is stopped.

The `abort_result` frame says what was stopped and whether another `abort`
would have stopped something more at the moment the stop was recorded.

A stopped action records the stop itself. It writes all the input waiting
for its next boundary: the human steers still pending, the agent messages and
the command reports. It completes each running tool call as an error
that says why and what became of its command
([Interrupted calls](shell-tool.md#interrupted-calls)), keeping in the call's `view` the
command it started, so the command's later report and notice find their call,
 and appends an `abort` block, the
stopped marker. An action that has written nothing into the transcript, with the
input its stop writes counted, appends no marker: it began no turn, so
nothing is left to continue
([The unfinished turn](failures-and-recovery.md#the-unfinished-turn)). Only a
send stopped while it still waits for the tree's admission
([Actions](#actions)) has not written its message; it leaves like a stopped
queued message, and the page still holds it as an unconfirmed submission
([Persistence and adapters](../product/web-application.md#persistence-and-adapters)).
If the action was saving a history rewrite, it records the stop after the
rewrite is published, so a rewrite never loses a stop. `abort_result` is sent
only once the record is in the transcript. A provider run that is cancelled
ends without an event; the session, not the provider, records the stop.

A Stop stops only what the node is doing at that moment, and holds nothing
afterwards, whether or not it wrote a marker:

- The node's subagents run on. Stopping them is a request of its own
  ([Abort](subagents.md#abort)).
- An agent message that arrives after the Stop wakes the node as it wakes an
  idle one ([Delivery and scheduling](subagents.md#delivery-and-scheduling)).
- A command report that arrives after the Stop wakes the node as it wakes an
  idle one ([Command reports](shell-tool.md#command-reports)).

For example, the user stops a turn while a subagent is still reading files.
The turn ends with the stopped marker, the subagent finishes, and its
completion opens a continuation of the root that reads it.

### Dispose and restore

A session is disposed when its tree closes: a `close` frame, the eviction of an
idle detached tree ([Frame protocol](rendering.md#frame-protocol)), or backend shutdown.
Dispose does the following:

1. Refuses new actions.
2. Stops the running action as a shutdown. The human steers still pending
   are written, running tool calls complete as interrupted by the shutdown,
   their commands still running
   ([Interrupted calls](shell-tool.md#interrupted-calls)), and an `error` block with the code
   `interrupted` and the message "The agent session was shut down while this
   turn was running." says why the turn is unfinished. A message whose turn
   has not written its `user` block yet is not interrupted: it goes back to
   the front of the queue.
3. Keeps the queued messages and the agent messages and command reports
   waiting for a boundary in the checkpoint.
4. Waits for a save in progress, then writes the final checkpoint. Its phase
   is `running` when a turn was interrupted.
5. Closes the provider runtimes, including the one a pending model switch had
   built.

Restoring reads the checkpoint ([Tree store](#tree-store)). A tool call still
marked executing completes as an error
`Tool call interrupted: the backend stopped before its result was recorded`,
followed by its command's number when it had one, which the backend takes up
again when its runner connects
([Recovery and persistence](../execution/sessions-and-targets.md#recovery-and-persistence)):
its outcome is unknown, and it never runs again
([Recovery and persistence](../execution/sessions-and-targets.md#recovery-and-persistence)).
The restored session is idle. A session restored from an interrupted
turn holds its waiting input until the node's next action.
Until then its transcript ends with the interruption record, so a later
restart holds them again. It hands back its queued messages and
whether a turn was interrupted; the node's lifecycle policy decides what
happens next ([Persistence](subagents.md#persistence)). A root leaves its interrupted turn to
its client, which the product offers as Resume
([Recovering an unfinished turn](../product/turns.md#recovering-an-unfinished-turn)).
When the transcript does not yet end with an interruption record, because the
process died instead of shutting down, the root appends one and saves it at
once.

### Model switch

A switch changes the root's model selection. The backend switches the root
when the conversation's model settings change, and opens a tree with the
selection the conversation's record holds
([the conversation's model settings](../product/web-api-conversations.md#sidebar-mutations-and-read-state)).
No client frame names a model.

- A switch lands at the root's next provider request: at the start of the
  next action, once the action has written its input, or inside a running
  turn at its next continuation boundary. A send's `user` block and a
  continuation's command reports or agent messages are written first, so a switch
  that fails to compact, or is stopped while it compacts, ends a turn that
  holds the input ([The unfinished turn](failures-and-recovery.md#the-unfinished-turn)).
  A request that is streaming finishes with the model it started with. A
  restored root keeps its checkpoint's model until its next action starts.
- While an edit is being prepared, a switch waits, and lands once the edit is
  accepted or rejected. The replacement turn starts with the model it was
  prepared with and switches at its next continuation boundary, so the edit
  never discards a switch that arrived meanwhile
  ([Message editing](message-editing.md)).
- When the history is over the new model's compaction threshold, the session
  first compacts with the current model and provider, then switches
  ([Compaction](compaction.md#compaction)). A switch that lands inside a
  running turn and compacted appends a `resume` block.
- A switch to another provider builds a new provider runtime. The replaced
  runtime is closed once no run uses it, and so is the runtime of a pending
  switch that a switch to yet another provider replaced. A switch within the
  pending switch's provider builds nothing and lands on the pending switch's
  runtime: a user who picks another provider's model and then another model
  of that provider before sending gets that provider's runtime.

Every block records the model selection that was current when it was created,
so a switch changes only later blocks.

## Input

For example, while the agent runs `npm test`, the user sends "skip the flaky
e2e suite" as a steer. The session accepts it at once, and the page shows it as
pending above the composer. When the command's result comes back, the session
writes the steer into the transcript as a `steer` block, and the next provider
request carries it. The page drops the pending entry when the block arrives.

A session takes three kinds of input:

- A message (`send`) starts a turn. While an action runs, it waits in the
  queue.
- A steer adds input to the running turn at its next continuation boundary.
- An agent message comes from another agent of the tree. It enters the
  transcript at the same boundaries as a steer and, while nothing runs, opens
  one continuation; its rules are in [Communication](subagents.md#communication).

A continuation boundary is a point in a turn where waiting input is written
into the transcript:

- before each provider request;
- after the provider's stream ends, unless the turn is about to compact;
- after each step of a round's tool calls ([Dispatch and failures](shell-tool.md#dispatch-and-failures)),
  unless the turn is about to compact.

A human steer is written only at the first of these, just before the request
that carries it, never after a stream or a tool step: until the model is
about to read it, it stays pending, so the page keeps showing it above the
composer with *Will be sent to the agent at its next step*, Send Now and
Recall ([Steer or queue](../product/turns.md#steer-or-queue)). For example,
a steer sent the moment the model asked for a five-minute build used to be
written at once, below the build's call, as a plain message with nothing
running after it, and the model read it only when the build's window ended;
now it stays pending above the composer, where Send Now ends the window if
the user does not want to wait. Agent messages and command reports keep all
three boundaries.

No provider receives input while it streams: a steer always waits for the next
boundary. When input arrived during a round, the turn asks the provider once
more, even when the model had finished or a tool had asked to end the turn.

### Messages and the queue

- A `send` carries a message id chosen by the client. The id becomes the
  `user` block's turn id and the queued message's id. A send whose id is
  already the running turn, a queued message or the turn of a `user` block is
  acknowledged without a second turn, so a client can repeat a send whose
  delivery it could not confirm
  ([Persistence and adapters](../product/web-application.md#persistence-and-adapters)).
- Only messages appear in the queue, never continuations or other actions. A
  `queue` frame carries the whole queue, each entry `{ id, content }`; the page
  derives the text it shows from the content.
- `dequeue_message` removes one message. `send_queued_message` sends one now
  ([Send now](#send-now)): while a turn runs, it ends that turn at once and
  the message runs next; otherwise the message moves to the front of the
  queue and runs next. A queued message never becomes a steer.
  `clear_message_queue` removes them all.
- The queue is part of the checkpoint. A change to the queue is saved even when
  the transcript does not change, dispose keeps the queue, and a restored
  session sends the queued messages in order.

### Steers

- The session accepts a steer while an action is preparing, streaming, running
  tools or compacting. It refuses a steer when nothing runs, when the running
  action was stopped or is finishing, and while an edit is being prepared.
- An accepted steer is pending until the next continuation boundary, where it
  becomes a `steer` block with the steer's id.
- Stopping an action writes its pending human steers before the stopped
  marker ([Stop](#stop)). A failed action writes all pending input before it
  ends. A human steer still pending when its action ends normally is
  discarded; agent messages are kept for the next continuation.

### Pending steers

The session keeps the list of accepted human steers that are not yet in the
transcript. Each entry holds:

| Field | Meaning |
| --- | --- |
| `id` | The steer id from the `steer` frame. The `steer` block uses the same id. |
| `turnId` | The running turn that receives the steer. |
| `model` | The model selection when the steer was accepted. |
| `content` | The steer's content, attachments included. |

The list never contains command reports or agent messages.

- The server sends a `pending_steers` frame with the complete list when a
  client opens the conversation, after the transcript, phase and queue frames,
  and again whenever the list changes: a steer is added, canceled, written to
  the transcript, or discarded.
- `steer_result` answers a `steer` frame with its outcome: accepted, or
  rejected with a reason. It does not replace the list notification.
- `cancel_pending_steer` removes a steer that is still pending, and
  `dequeue_message` a queued message; each is answered by `remove_result`
  with its id and whether it was removed or had already gone on: a steer the
  turn took into its transcript, a message whose turn started, or an id the
  session never held. The answer follows the list's own frame, so the page
  never infers the outcome from what comes next.
- Human pending steers are not part of the checkpoint. A backend restart loses
  them; they never reached the transcript. The backend keeps no second copy of
  them for reconnecting clients.

`ConversationClient` keeps the current list and exposes it with its `pendingSteers()`
reader and `pending_steers` events. It removes an entry once a `steer` block
with the same id appears in the transcript; equal text does not make two
steers the same. Closing or replacing the client's session clears the list.
`cancelPendingSteer(id)` and `dequeueMessage(id)` resolve with the session's
`remove_result`. A request that reaches no session, one the backend could not
open, is answered `gone_on`, since nothing held it.

A client can reconnect to a running session without losing a pending steer.
Suppose the session has accepted steer S1 but has not written it yet, and a new
client opens the same conversation. The server attaches the new client to the
live tree and includes S1 in the initial list. S1 is not sent to the model a
second time. When S1 enters the transcript, the client stops listing it.
`ConversationClient.open()` resolves on the `opened` frame; the snapshot frames follow
it, so a caller that needs the initial list subscribes to `pending_steers`,
which also reports an empty list.

### Send now

For example, the agent runs `npm test` with a ten-minute window, and two
minutes in the user steers "skip the e2e tests". The steer waits for the call:
it would reach the model only when the call returns, up to eight minutes
later. The user presses Send Now on it. The call returns at once with the
output so far and a line saying that the command moved to the background
because the user sent a message; the command keeps running. The steer enters
the transcript, the turn goes on, and the model can stop the command and run
it again without the e2e tests. Had the user pressed Send Now on a queued
message instead, the turn would have ended at the same point and the message
would have started the next turn.

A human steer waits for the next continuation boundary, and a queued message
waits for the end of the turn. Sending one now delivers it at once without
stopping anything the agent started, and without changing what it is: a steer
stays a steer, and a queued message starts a turn of its own.

- `steer_now { steerId }` names a pending steer of the running turn.
  `send_queued_message { messageId }` names a queued message
  ([Messages and the queue](#messages-and-the-queue)).
- The session cuts the running round short:
  - a streaming provider request, or the wait before its retry, is cancelled,
    and what it streamed stays in the transcript, as with [Stop](#stop);
  - each running shell call returns at once with the result its window's end
    gives ([The window](shell-tool.md#the-window)), preceded in its hint by the line
    `[The user sent a message, so command 17 moved to the background. It
    keeps running.]`; its command keeps running;
  - a requested call that has not started completes as the error `Tool call
    not run: the user sent a message`.
- Nothing is stopped and no `abort` marker is written. A compaction or a save
  in progress is not cut: the session sends now once it ends.
- At the boundary the cut reaches, the session writes all waiting input into
  the transcript as any boundary does: the pending human steers, the agent
  messages and the command reports. After `steer_now` the turn goes on with the
  next provider request. After `send_queued_message` the turn ends there, and
  the message runs next, ahead of the rest of the queue. A queued message sent
  now before the turn's first request ends the turn at once, so the message
  that started it is not answered on its own.
- When no turn runs, `send_queued_message` moves the message to the front of
  the queue, and it runs as soon as the running action, if any, ends.
  `steer_now` for a steer that is no longer pending changes nothing.
- Neither frame has a reply; the client observes the transcript, the
  `pending_steers` list and the queue.

## Identifiers the model sees

Every identifier the model reads or writes is short: a number, or a letter
and a number, given in order within the scope the model uses it in, and never
given twice there. For example, the seventeenth command of a conversation is
`17`, whichever agent ran it, and `demi shell output 17` reads it. A random
UUID in its place costs about 20 tokens in every result and every request
that replays it, and a short number is copied without a slip.

| Identifier | Looks like | Unique within | Given out by |
|---|---|---|---|
| A command (`commandId`) | `17` | The conversation | The backend, when the command starts |
| A command's medium | `2` | Its command | The runner, as the medium reaches it ([Media the model views](shell-tool.md#media-the-model-views)) |
| An agent | `0` for the root, then `1`, `2`, … in spawn order | The conversation | The backend, when the agent is spawned ([Model-facing surface](subagents.md#model-facing-surface)) |
| An agent's round | `1` for its first run, one more at each resume | The agent | The agent's supervisor |
| A conversation browser tab | `t7` | The conversation | The backend, when the tab is registered ([One tab registry](../browser/browser.md#one-tab-registry)) |
| A simulator `demi mobile` opened | `m2` | The conversation, across its Hosts | The backend, when the device is opened ([Commands](../mobile/mobile.md#commands)) |
| An attachment the agent uploaded | `a3` | The conversation | The backend, when `demi attachment upload` stores it ([Attachment commands](../execution/commands.md#attachment-commands)) |
| An element reference | `e37` | Its tab, or its device | The conversation browser, as it observes the tab, or `demi mobile inspect` |
| A host | Its name, as `demi host list` shows it | The conversation's hosts | The user ([Attached hosts](../execution/sessions-and-targets.md#attached-hosts)) |

- **Never twice.** A number is not given again after a crash, a restore or a
  Fork either: the backend stores the next number of each sequence before it
  gives one out, so a crash can only leave a gap, and a Fork's destination
  goes on from its source's numbers, since its history names them
  ([Storage](../backend/storage.md#conversation-state-and-transactions)).
- **Only what the model uses.** The model's text carries no identifier it has
  no use for: a command report names no report, an agent message names no
  message id, and a missing medium is named by its kind, not by its blob's
  hash.
- **Paths stay whole.** A path keeps the identifiers it is made of, since
  several backends and users can share one machine: an attachment's path on a
  Host names its conversation's full id
  ([Attachments](../product/composer.md#attachments)).
- **Inside Demi**, an identity keeps the form its storage and its wire need;
  this rule is about the text the model reads and writes.

## Tree store

The tree store is the persistence contract between the agent and a
conversation's database. One store holds one conversation's tree: every node
with its parent link and its checkpoint. The backend realizes it over the
conversation's database
([Conversation state and transactions](../backend/storage.md#conversation-state-and-transactions)).
Tests use an in-memory store that meets the same contract. A node's store also
gives its session the conversation owner's blob namespace, where the session
puts the media that enter its transcript and reads back those its requests
send ([Media](transcript.md#media)): a put names bytes by their SHA-256, and a get answers
a blob's bytes or that the namespace does not hold it.

A node's checkpoint has two parts:

| Part | Holds |
| --- | --- |
| Transcript rows | One row per block, by index |
| State row | The phase; the queued messages, each `{ id, content }`; the agent messages and command reports waiting for a boundary; each running command's interval; the working directory; the model selection; the accepted edit receipts |

Human pending steers are not part of it. Creating,
closing, reopening and deleting nodes, and delivering subagent completions, are
atomic commits of the same store ([Persistence](subagents.md#persistence)).

### Saving

- A save carries only what changed: the changed block rows in ascending index,
  the block count (rows at or beyond it are deleted), and the state row. The
  store commits a save in one transaction.
- A session has one save in progress at a time. Saves, history rewrites and
  edit commits run in the order they were requested. A running turn never
  holds this order while it waits for a provider, a tool or a context source,
  so the scheduled saves of its progress commit meanwhile.
- A save that has started always finishes: Stop does not cancel it, and it
  took effect exactly when the store reports success.
- A change schedules a save one second after the first unsaved change. These
  points save at once: before tools are dispatched, after a `context` block,
  when an action ends, when an agent message is admitted or written, on a
  history rewrite or an edit commit, and on dispose. The save at the end of an
  action records the phase idle, so a checkpoint that says an action was
  running is one the process died in; clients see the phase go idle only once
  that save has committed ([A turn](#a-turn)).
- A save is due when the transcript, edit receipts, the queue,
  the waiting agent messages or the model selection changed. The phase alone
  never makes a save due.
- A scheduled save that fails is reported as an `error` frame, and its rows are
  saved with the next change. When the save at the end of an action fails, the
  action fails.
- A history rewrite is saved before it is published: the store commits the
  retained rows, then the session adopts them and publishes the rewrite's
  batch of patches ([Patches and versions](transcript.md#patches-and-versions)).
- A save writes its rows as they are. Their media are references whose blobs
  were stored when the media entered, and a block or a queued message has no
  place for bytes ([Media](transcript.md#media)), so a save stores no blob.

### Restoring

- The store decodes and validates every row it reads, and corrupt data stops
  the restore ([Storage](../backend/storage.md)).
- The session adds its own checks: the waiting agent messages have unique
  ids that are not already in the transcript, each is addressed to this node,
  and edit operation ids are unique.
- The session then completes interrupted tool calls and hands back its queue
  as [Dispose and restore](#dispose-and-restore) describes.
- A restore reads no blob: the session reads the blobs of its replayed media
  before its first request ([Media](transcript.md#media)).

## Acceptance

Tests use scripted providers, in-memory or SQLite stores, and a real runner
where a tool runs; no test calls a real model.

| Situation | Required observation |
| --- | --- |
| A client reconnects while a steer is pending | The initial `pending_steers` lists it; the model receives it once; the client drops it when its `steer` block arrives |
| A steer arrives while a tool runs | The next provider request carries it and no earlier one does; a canceled pending steer causes no extra request |
| A tool is about to run | The store already holds its call as executing; after a simulated crash, restore completes the call as interrupted and does not run it again |
| Stop during a stream | `abort_result` arrives after the stopped marker is in the transcript; the stopped send's action ends as stopped, not failed |
| Stop while resume saves its unwind | The transcript holds the unwind and one stopped marker, and no `resume` block without a turn |
| Dispose during a turn | The final checkpoint has the phase `running`, the interruption record, the aborted tool calls and the queued messages |
| A turn's closing save is held at its commit | The client sees no `idle` until the save commits; an edit it sends the moment it sees `idle` is admitted |
| A message is queued while a tool runs | The queue is saved without any transcript change |
| Two opens of one conversation at once | One live tree |
| Two connections of one conversation | Both receive the same events of a turn; a reply reaches only the connection that asked; a `close` sends `closed` to both |
| One of two connections stops reading | It alone closes as lagging; the other receives the whole turn |
| Two connections act at once | Two sends run in the order they arrived; a send while the other's edit is prepared is refused, and so is an edit while the other's send waits or runs |
| A command prints while its `shell` call runs | The attached connections receive the output before the call's result |
| A command prints after its call returned | Every attached connection receives the new output, with no frame from any client |
| A connection attaches while a command runs | Its handshake carries the command's view, whatever another connection or the model read; it then receives each change as the others do |
| A command prints faster than a page reads | A page receives at most one frame of it every 250 ms, and its outbox does not fill; the view shows the output beyond the first 8 KiB of each stream |
| A command ends | No frame of it follows the frame of its end |
| The last connection detaches | The runner sends only each stream's newest 8 KiB beyond its first, every 2 seconds, and the tree sends nothing, until a connection attaches again |
| A command prints only beyond the first 8 KiB of its streams while no page is attached | The model's `idleMs` counts from its latest output, within 2 seconds |
| A model switch while a turn runs | The request in flight keeps its model; the turn's next request carries the new model, effort and tier |
| A model switch while an edit is being prepared | The switch is not refused; the replacement turn's first request carries the model it was prepared with, and its next request the new one |
| A client stops reading | The connection closes as lagging; a reconnect adopts the running tree and its turn completes |
| Frames of an open | The handshake order above; patch revisions increase by one; a stale patch after a reset is ignored; a gap triggers `sync_transcript` |
| Scripted tool events | The `shell` tool renders with its own component and its `description` title; updates replace the block in place; an unknown tool name renders as a generic card |
| A tool's result carries an image, a video, or a medium that is gone | The page shows each under the call's row, the media loaded from the blob route and a gone medium as what it was and why it is gone; a click on the image opens it large |
| Tool calls | Input refusals, the repeat guard, the cut of a result, handle release and binary stdout verdicts match [Tools](shell-tool.md#tools) |
| A loop of three `demi browser screenshot … \| demi file view` | The output holds each screenshot's description and its line `[image n: …]` in order; the result attaches the three images after the output, in that order |
| `demi browser screenshot t1 > shot.png` | `shot.png` holds the PNG; the output is the screenshot's description and the result attaches nothing |
| `demi browser screenshot t1 \| convert - -resize 50% png:- \| demi file view` | The result attaches one image, the half-size one |
| `cat shot.png` | The output's line `<binary stdout: …>`; the result attaches nothing and its line names `demi file view` |
| `x=$(demi file view a.png)` and `demi file view a.png > /dev/null` | The image is attached either way; `x` is empty |
| `demi file view notes.txt a.png` | `a.png` is attached; the line for `notes.txt` names `cat`, and the status is 1 |
| `demi file view` with no path and the job's own stdin | Status 1 with the message naming a file or a pipe; nothing waits for input |
| `demi file view report.pdf` for a model whose provider carries no document in a tool result | Status 1, the message naming the model and the type |
| Two background views and `wait` | Both attached, numbered in the order they reached the runner |
| A job views 25 small images | The result attaches the first 20 and gives a line for each other saying to view it in another call |
| A job views three images whose base64 together exceeds half the model's body limit, the second the largest | The first and third are attached; the second's line says why not |
| The model switches to one that does not read WebP after a job viewed a WebP image | That job's result says the image was not attached, and why |
| A job views its 33rd medium | Its line reads that it was not kept; the command goes on and succeeds |
| A job prints 20 MiB and then views a screenshot | The screenshot is attached |
| A job views a medium and is still running when the call's window ends | The `shell` result attaches nothing; the end report attaches the medium, also after a `demi shell status` showed the end, which shows only the medium's line |
| The Host's connection is lost after a job viewed a medium | The result that reports the end says the medium was lost with the connection |
| A command prints 200 KB of lines and exits | Its result shows whole lines from the start and the end and the line naming the lines between and `demi shell output 17 --lines`, and fits the replay bound, so every request carries it unchanged; the pages that command prints hold those lines, numbered, with the next page's command, and `--raw` prints the 200 KB in the order the runner read them |
| A page of a command's output | At most 12,000 characters of whole lines, so the result that holds it is not cut; a line over 2,000 characters shows its start and how to read it whole |
| `--raw \| grep -n` on a command's output | The numbers it prints select the same lines with `--lines` |
| A command exits having printed 25,000 characters, more than the replay bound | The result's line names `demi shell output`, which prints the characters the result leaves out |
| The same command for a model with a context window of a million tokens | The same result |
| A running command's stdout goes beyond its first 8 KiB while no page is attached | Its result shows the stream's start, the line counting the bytes left out, and its newest lines from the last 2 seconds |
| A command prints 20 MiB | The Host keeps 16 MiB of it while it runs, and the backend stores the same; `demi shell output` prints the first and last 8 MiB with the line where the rest was left out |
| A binary stdout | The result says it is not shown and names `demi file view`; `demi shell output 17 --raw --stdout` prints the bytes unchanged |
| `demi shell output 17 --raw \| head -n 1` | The line, and nothing on stderr |
| A subagent reads the output of a command its parent ran | The whole output |
| A stored conversation with images is opened by two pages, and one asks for the transcript again after a gap | No blob is put: every frame carries the references its rows hold |
| A restored conversation with images before and after its last `compaction_boundary` runs a turn of two requests | The first request reads the blob of each replayed medium once and none from before the boundary; the second reads none; both carry the replayed media's bytes |
| A tool's medium cannot be stored | Its result holds the medium as gone, not stored, with the store's error; the model receives `[<kind> not stored: <reason>]`, and the turn goes on |
| A replayed medium's blob is missing | The model receives `[missing <kind>]` in its place, in every request of the live tree, and the turn goes on |
| One scripted conversation with tools, images, thinking, a steer, a subagent's result and a command report, for each provider, Codex over each of its transports | Each request's body begins with the previous request's body, byte for byte apart from the Anthropic cache marks, which the vendor does not count as content; each exception of [The rule](../providers/providers.md#the-rule) changes only what that rule names; Codex's WebSocket message is its server-sent events body, byte for byte, after the message's type |
| A switch to a model that does not accept video | Every request to it carries the history's videos as the same text |
| A video whose base64 is over half of a model's request body limit | A tool result does not attach it, and replay sends one already in the history as the same text in every request to that model |
| An image over 2,000 px enters from a tool result and from an upload | It enters fitted, every later request carries the same bytes, and the original stays on the Host |
| A PNG illustration of 2 MB and a PNG screenshot of 150 KB enter from tool results | The illustration enters as a smaller JPEG and the screenshot unchanged; later requests carry the same bytes |
| Images bring a request over a byte or an image limit the scripted provider sets, with the estimate under the token threshold | No compaction runs; the vendor's refusal fails the turn with `request_too_large`, and `resume` can continue it |
