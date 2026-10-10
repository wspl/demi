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

This document owns sessions and turns, their input, command reports, the
`shell` tool, the transcript and its views, the rendering boundary, the frame
protocol, and the tree store contract. Related rules have their own homes:

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
| The command set | The commands every node starts from; the runtime adds its own groups per node ([Tools](#tools)) | The commands of the plugins the user has on, and the `demi host` and `demi attachment` groups ([Plugins](../architecture/plugins.md#commands)) |
| Instructions | The identity that opens the system prompt ([System prompt](system-prompt.md)) | The product's instructions; a plugin adds only its command groups' index entries ([Prompt text and context](../architecture/plugins.md#prompt-text-and-context)) |
| Subagent settings | Whether subagents are on, and the named [subagent profiles](subagents.md#profiles) with whether each is enabled, as data, asked at each spawn and each `demi agent profiles` | The user's current settings, which the user edits in the Subagent section of settings |
| Provider runtimes | A node's model selection and the runtime that serves it: the root's from the conversation's record, a child's from its profile's model settings or its parent's ([Runtime](subagents.md#runtime)) | The backend's provider assembly ([Inference admission and runtime ownership](../providers/providers.md#inference-admission-and-runtime-ownership)) |
| The Host of a node | Where its shell tools run now, asked at each shell tool call | The conversation's host access ([Host operations](../execution/sessions-and-targets.md#host-operations)) |
| Context sources | What the model must learn before a request, asked before each one ([Context](#context)) | The conversation's execution context, then each plugin that is a context source, while the user has it on |

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
  |   compact first when the request reaches a size threshold
  |   request the provider, retrying transient failures
  |   refused as too large: compact once and request again
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
before a request that would be too large and after one that was refused, is
described in [Compaction](compaction.md#when-compaction-runs).

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
([Frame protocol](#frame-protocol)).

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
([Interrupted calls](#interrupted-calls)), and appends an `abort` block, the
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
  idle one ([Command reports](#command-reports)).

For example, the user stops a turn while a subagent is still reading files.
The turn ends with the stopped marker, the subagent finishes, and its
completion opens a continuation of the root that reads it.

### Dispose and restore

A session is disposed when its tree closes: a `close` frame, the eviction of an
idle detached tree ([Frame protocol](#frame-protocol)), or backend shutdown.
Dispose does the following:

1. Refuses new actions.
2. Stops the running action as a shutdown. The human steers still pending
   are written, running tool calls complete as interrupted by the shutdown,
   their commands still running
   ([Interrupted calls](#interrupted-calls)), and an `error` block with the code
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
([Recovering an unfinished turn](../product/product.md#recovering-an-unfinished-turn)).
When the transcript does not yet end with an interruption record, because the
process died instead of shutting down, the root appends one and saves it at
once.

### Model switch

A switch changes the root's model selection. The backend switches the root
when the conversation's model settings change, and opens a tree with the
selection the conversation's record holds
([the conversation's model settings](../product/web-api.md#sidebar-mutations-and-read-state)).
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
- after each step of a round's tool calls ([Dispatch and failures](#dispatch-and-failures)),
  unless the turn is about to compact.

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
- `cancel_pending_steer` removes a steer that is still pending. It has no
  reply; the client observes the resulting list and transcript. An id that is
  not pending changes nothing.
- Human pending steers are not part of the checkpoint. A backend restart loses
  them; they never reached the transcript. The backend keeps no second copy of
  them for reconnecting clients.

`ConversationClient` keeps the current list and exposes it with its `pendingSteers()`
reader and `pending_steers` events. It removes an entry once a `steer` block
with the same id appears in the transcript; equal text does not make two
steers the same. Closing or replacing the client's session clears the list.
`cancelPendingSteer(id)` sends the request and returns; the caller watches the
list and the transcript for the result.

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
    gives ([The window](#the-window)), preceded in its hint by the line
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

## Command reports

For example, the model starts the test suite, command 17, with
`intervalMs: 300000`. The suite runs on after five minutes, so the call
returns its handle and output, and the model tells the user that the tests
are running and ends its turn: the user can talk to the agent meanwhile.
Five minutes later the node is woken with a report, "Command 17 is still
running; no output for 4 minutes 50 seconds", and the newest lines. The model
sees that the suite hangs in one test, stops it and runs that test alone.
Had the suite ended first, the node would have been woken with "Command 17
(Run the test suite) ended with exit code 1" and the suite's last lines, as
the call's result would have shown them. The model never
asks to be woken: as Claude Code wakes its model when a background command
or a subagent ends, the work it waits for wakes it.

- **Every command says how it is watched.** `shell` takes
  `intervalMs`, a number or `null`, and no default, so the model decides when
  it starts the command:
  - **A number**, from 15,000 to 600,000 milliseconds, for a command that
    ends, such as a build or a test suite: short for one that should end
    within minutes, long for one that takes long, since each report is a
    request of the whole conversation. The call watches the command for
    up to that long; if it still runs, the call returns its handle, and the
    node then gets a report every interval until the command ends. A report
    carries what `demi shell status` would show: the output since the
    model's last look, how long the command has printed nothing, and how to
    change the interval. A smaller value is taken as 15,000 and a larger one
    as 600,000, and the result says so.
  - **`null`** for a command that runs until it is stopped, such as a dev
    server or a watcher. The call returns once the command's output has been
    quiet for 2 seconds, or after 30 seconds, so the model sees it start,
    and the node gets no reports while it runs.
  - `demi shell status --interval` and `--resident` change a running
    command's interval from then on: longer when a report shows the command needs
    more time, `null` when the model learns that a command it started with a
    number serves, a number when it wants to hear from a resident one. A
    report says how, so the model changes it where it already looks at the
    command.
- **Every end is reported, with what the model would look at.** A command
  whose call returned while it still ran tells its node when it ends,
  however it ends: an exit with its status,
  a stop by the user or another agent, or a loss with its reason
  ([Lost commands](#lost-commands)). A command the node stopped itself with
  `demi shell stop`, which printed `[command 17 stopped]`, and one whose end
  the model saw in a result already, tell nothing more. An end report is the
  result that reports the command's end
  ([What a result attaches](#what-a-result-attaches)): it carries the output
  since the model's last look, bounded as a `shell` result is, and the media
  the job viewed, so the model needs no `demi shell status` to see how it
  ended: in a product check, a model woken by an end without its output
  spent one more request on a look every time.
- **How a report arrives.** A report joins a turn that accepts steers at its
  next continuation boundary, as a `wakeup` block with the placement
  `steer`; otherwise it starts a continuation whose input is a `wakeup` block
  with the placement `new_turn`. Each report names the command by its
  number and its call's title, as Claude Code's notice names a background
  command by its description, since a model may have several running:
  `Command 17 (Run the test suite) ended with exit code 1.`, followed by its
  output as a result shows it. Reports that arrive together are one block,
  one paragraph each: `Command 18 was stopped by the user.`,
  `Command 19 was lost: Demi was upgraded and the Host's runner replaced
  itself. Start it again if it is still needed.` A report never appears in
  the queue or among the pending steers, and a running call's window ends
  when one arrives ([The window](#the-window)), except the end report of a
  command that call watches or looks at, whose end the call shows itself. A
  report moves the model's place in the command's output as a look does,
  when it is written into the transcript; an end report dropped because a
  look showed the end first moves nothing, so the look shows the last
  lines. An end report whose job viewed media is never dropped: it carries
  the media, without the output the look showed.
- **A Host that is away.** A command whose Host's connection is away sends
  no interval report: the node could do nothing with one, and its silence
  is the connection's, not the command's. Its reports resume when the
  runner is back, and its end, or its loss when the 10 minutes pass, is
  reported as any end ([Command lifetime](../execution/runner.md#command-lifetime)).
  A look at it with `demi shell status` from another Host says that its
  Host is unreachable, and since when.
- **Across a restart.** A command's start time and each node's place in its
  output are part of the record the backend keeps of a running command, so
  a report after a backend restart counts the time from the command's
  start and shows only output the model has not seen.
- **What the user sees.** The transcript shows each report as a row where
  its block lies, as an agent message shows as its receipt row, so a reply
  the agent writes after it never appears without its cause, as Claude Code
  shows a line when a background task ends. The row names the call's title
  and, set apart from it, what happened, in the user's words: *Run the test
  suite* · *ended with exit code 1*, *Start the dev server* · *stopped by
  you*, *Run the test suite* · *still running*. A title is an imperative,
  so joined into one sentence it read wrongly in the product: *Restart the
  end-to-end suite is still running*; a failure, a stop and a loss carry the same tag a shell row
  carries ([Rendering boundary](#rendering-boundary)), and a click opens the
  command's terminal tab. So the block holds each report as data, its
  `commandId`, the call's `title`, its `event` (`running`, `ended` with
  its exit code, `stopped` with who stopped it, or `lost` with its reason),
  and the output and media it carried, and the text the model reads is
  rendered from them in one place, never stored beside them. A report that carries media shows them under its row,
  as a call's result does
  ([Media a tool returned](../product/file-previews.md#media-a-tool-returned)).
- **Waiting is ending the turn.** A model that has nothing left to do but
  wait for a command or a subagent ends its turn, with a reply that says
  what it waits for, since the user reads only what the model wrote. The
  tool rules say so, and say not to poll. To come back after a while for
  something no command stands for, such as a deployment that becomes
  visible minutes later, the model starts `sleep 600` with an interval and
  ends its turn; a command that must end by a deadline runs under `timeout`
  ([Background tasks and timeouts](../execution/runner.md#background-tasks-and-timeouts)).
- **Restarts.** A report that has arrived but is not yet in the transcript
  is saved in the checkpoint, as agent messages are, and so is each running
  command's interval, so both survive dispose and a backend restart. A
  command that reports while its conversation's tree is not live, such as
  one the backend took up again after a restart
  ([Recovery and persistence](../execution/sessions-and-targets.md#recovery-and-persistence)),
  restores the tree, as an `open` does, to tell its node. A root whose last
  turn was interrupted holds its reports as it holds its agent input, until
  the user resumes it.
- **Children.** A subagent whose background commands still run stays live,
  as one with live children does, and closes once they have ended and it
  has answered ([Result](subagents.md#result)).

## Tools

The model has one tool, `shell`, and only it:

| Tool | What it does |
| --- | --- |
| `shell` | Starts a script in a shell on the conversation's Host and watches it as its `intervalMs` says ([Command reports](#command-reports)). The window is for watching, not a deadline: a command still running when it ends keeps running, the result carries its handle (`commandId`), and the node hears from it again as its interval says. Completed short output returns directly. |

A tool is only what the shell cannot do: start a script and watch it. Looking
at a command again, writing to its input and stopping it are things a shell
does, so they are the commands `demi shell status`, `demi shell input` and
`demi shell stop` ([The `demi shell` commands](#the-demi-shell-commands)),
which a script can combine with anything else. Ending a turn needs no tool,
since a response that calls none ends it, and waiting needs none, since the
work's end wakes the node ([Command reports](#command-reports)). Earlier
designs had three tools: `shell_status` to look at a command, which a
command does as well, and `yield`, which ended the turn and set a timer;
models used the timer to wait for work that wakes them anyway, and a timer
that fired after the work had ended opened a turn that only said so. Claude
Code likewise has no tool to look at a background command: its output is a
file the model reads like any other.

Everything else the agent does runs as commands in the shell
([Commands](../execution/commands.md)). The runtime grafts its own groups,
`demi agent` ([Subagents](subagents.md)) and `demi shell`
([The `demi shell` commands](#the-demi-shell-commands)), into each node's
command set; the others, such as `demi file`, `demi browser` and `demi
host`, come from the command set the product supplies. A tool call whose name
is not `shell` completes as an error `Tool not found: <name>`.

### Tool input

- Each tool declares its input once. The JSON Schema the model receives and the
  check each call runs come from that one declaration. The schema's dialect
  declaration is dropped, because a provider embeds the schema in its own
  request.
- Unknown fields and wrong types are refused. A refused call completes as an
  error whose text starts with `<tool> input is invalid:` and names each
  offending field, so the model can correct the call.
- `intervalMs` is required: whole milliseconds, or `null`, declared to the
  model as `integer`, whose description gives its bounds, 15,000 to 600,000.
  A value outside them is taken as the nearest bound, and the result's first
  line says so (`intervalMs 900000 is above the cap; taken as 600000`), since
  the command goes on either way: models asked for fifteen minutes for a
  test suite six times in one conversation, and a refusal only cost them a
  call each time. A fraction is refused, not rounded.
- `shell` takes a `description`, the call's title for the user, and requires
  it ([Tool descriptions](#tool-descriptions)).

### Running shell tools

- The shell tools reach the conversation's current Host through the
  conversation's host access
  ([Host operations](../execution/sessions-and-targets.md#host-operations)).
  Their jobs run in the runner's shell
  ([Shell jobs](../execution/runner.md#shell-jobs)).
- Every `shell` call starts a job of its own in the conversation's working
  directory on its Host, the directory the context block names
  ([Switch the primary target](../execution/sessions-and-targets.md#switch-the-primary-target)).
  Nothing of an earlier command carries over: not its directory, its
  variables or its functions. For example, after a script that runs
  `cd browse && npm test`, the next call starts in the working directory
  again, not in `browse`. Where a call starts is therefore never a guess: a
  carried directory made a model's next `cd ./browse` fail inside `browse`,
  and models began every script with `cd <workspace>` because the context
  block's directory stopped being true after the first `cd`.
- Each node keeps one shell environment per Host it has used, which holds
  its commands' handles. Concurrent calls for one Host create one
  environment.
- A handle belongs to its environment. A `commandId` of another Host's
  environment is refused with `Command "<id>" belongs to a different Host`,
  and a handle that two environments claim is refused as not unique.
- The repeat guard counts identical scripts. In one environment, a `shell` call
  of the same script within 60 seconds of the previous one is allowed six times
  in a row. The seventh and later identical calls do not run: the result is an
  error that asks the model to inspect the previous output, and its view is
  `repeated_shell` with the script and the count. Another script resets
  the count.

### The window

For example, the model runs the test suite with a ten-minute interval, and two
minutes in a subagent reports that the failing test is a flaky one. The window
ends at once: the call returns the running command's handle and its output so
far, the agent message enters the transcript, and the model can stop the
command and run it again without that test. Without that, the message would
wait up to eight more minutes.

A `shell` call watches its command until the first of:

- the command ends;
- the interval passes, or, for a command whose interval is `null`, its
  output has been quiet for 2 seconds or 30 seconds have passed
  ([Command reports](#command-reports));
- an agent message or a command report arrives, which joins the turn at its
  next boundary ([Input](#input));
- the user sends a steer or a queued message now, and the result says so
  ([Send now](#send-now)).

Ending the window never stops the command: it keeps running, and the result
carries its handle, as when the time passes. A human steer ends no window: by
steering rather than sending now, the user chose to wait for the running call.
A message sent to the queue does not join the turn, so it ends no window
either.

### The `demi shell` commands

The runtime's own command group works on the conversation's commands. Each
is an `rpc` command, handled in the backend from any of the conversation's
shells, so a script combines them freely: `demi shell input 17 <<'EOF'`
answers a prompt and `demi shell status 17` in the same script shows what
the command did with the answer.

| Command | What it does |
| --- | --- |
| `demi shell status <commandId>…` | Prints each command's status, its exit code once it has ended, how long it has run and printed nothing, and its output since the node's last look at it, as a result shows it ([Results and previews](#results-and-previews)), also of a command that was stopped or lost: `status: stopped` or `status: lost: <reason>`, then the output; it shows the media's lines and attaches none ([What a result attaches](#what-a-result-attaches)). A command whose Host's connection is away, looked at from another Host, says so: `status: running, as far as Demi knows; its Host has been unreachable for 40 seconds, and its runner keeps it for up to 10 minutes`. `--wait <duration>` waits up to that long for the commands to end first; the `shell` call that runs it watches as any call does. `--interval <duration>` or `--resident` changes how the node hears from the commands from then on ([Command reports](#command-reports)). |
| `demi shell input <commandId>` | Writes its stdin to the command's input, such as an answer to a prompt, and prints nothing; input for a command that is not running fails. A command that still acquires its Host, as while a Cloud wakes, takes the input once it starts. |
| `demi shell output <commandId>` | Prints the command's whole output ([The whole output](#the-whole-output)). |
| `demi shell stop <commandId>…` | Stops each command, below. |

`demi shell stop <commandId>…` stops each running command of the conversation it names,
whichever agent ran it, and waits until it has ended: it prints
`[command 17 stopped]`, and the command's next status shows it
`stopped` with its last output. A command that had already ended prints
`[command 17 had already ended]` and also succeeds, so stopping is safe to
repeat; a number that names no command of the conversation fails with
`demi shell stop: 17: no such command in this conversation`, while the
others it names are still stopped. It stops the command
as the page's stop does,
through the command's shell environment: `TERM` first, so the command and
every program it started can end cleanly, and `KILL` for what has not
ended after 5 seconds
([Cancellation and completion](../execution/runner.md#cancellation-and-completion)).
`status` and `stop` read any command of the conversation, whichever agent
ran it; the place a status moves is the calling node's own.

### Results and previews

For example, `npm test` prints 4,720 lines, 212,345 bytes, and exits with
status 1 within its call's window. Its result shows the first and the last
lines of the output, and between them one line that says which lines it
leaves out and how to read them:

```text
status: exited
exitCode: 1
commandId: 17
output:
<lines 1-158>
[... lines 159-4562 not shown (196545 bytes); read them: demi shell output 17 --lines 159-4562 ...]
<lines 4563-4720>
```

- The model keeps its own place in each command's output: a result shows the
  output since the model's last look at the command. What the pages are sent
  never moves it ([Live output](#live-output)). For example, a page shows a
  running command's new output; the model's next `demi shell status` still shows
  all of it.
- A result gives the command's status, its exit code once it has exited, its
  `commandId`, its timings while it runs, the output, and a hint
  for the next step while it runs or once it was stopped: that the node
  will hear from the command as its interval says, and that
  `demi shell status`, `input` and `stop` look at it, answer it and stop it.
  A command whose script has ended but whose background tasks still run
  says so, naming them, since a model that started `nohup server &` and saw
  its command still running took it for a hang:
  `the script has ended; its background task "PORT=18948 bun src/server.ts"
  keeps the command running, and stopping the command stops it`.
- A result whose command exited with a status other than 0, or was stopped
  or lost, is marked as an error to the provider (`is_error` on Anthropic's
  API), as Claude Code marks a failed command, so the model weighs it as a
  failure; its text is the same.
- A result's `idleMs` counts from the last time the command's output grew,
  also beyond the first 8 KiB of a stream: the runner reports such growth
  within 2 seconds even while no page follows the command
  ([Pipes and output](../execution/runner.md#pipes-and-output)). For example,
  a build that prints its 100th KiB of log lines a second ago shows an
  `idleMs` below 3,000, so the model does not take it for a hung one.
- The model need not cut a command's output itself: the result is already
  bounded and points to the rest. The tool rules say so, since piping into
  `head` or `tail`, which models did in half of their scripts, hides the
  command's exit status behind the pipe's, hides a long command's output
  until it ends, and turns a returned image into a fragment of its bytes.
- The output is the merged stdout and stderr since the model's last look, in
  the order the runner read them. It is shown whole when the whole result fits
  the replay bound of 16,000 characters
  ([Text bounds](compaction.md#text-bounds)). Otherwise the result shows as
  many whole lines from the start and from the end as fit in half each of what
  the bound leaves after the result's other lines, the line between them
  included, and that line names the lines left out, their bytes, and the
  command that prints them ([The whole output](#the-whole-output)). A single
  line too long for its half is shown in part, and the line between names the
  characters left out and how to read them a part at a time:
  `[... characters 7901-40311 of line 1 not shown; read them: demi shell output 17 --raw | sed -n 1p | cut -c 7901-19900 ...]`.
  So a result is cut once, where it is made, and replay sends it unchanged.
  The rule is the same for every model. Characters are Unicode scalar values
  ([Token estimates](compaction.md#token-estimates)).
- While a command runs, the backend holds the first 8 KiB of each of its
  streams and, once a stream has gone beyond them, its newest 8 KiB, as the
  runner sent them within the last 2 seconds; what lies between stays on the
  Host ([Pipes and output](../execution/runner.md#pipes-and-output)). The
  result of a running command shows its output since the model's last look
  within the first 8 KiB, then, for each stream that has gone beyond them, a
  line that counts the bytes left out and the stream's newest whole lines,
  each part within half of the bound: for a dev server that has logged for an
  hour, its start, `[... 1040384 bytes of stdout not shown; its newest lines follow ...]`
  and its last requests. The model's place moves only within the first 8 KiB,
  so the next look shows the newest lines as they are then.
- The model's place moves past the whole lines a result covers, the lines
  it leaves out included. An unterminated line is shown at once, and the
  next look shows it again from its start, as far as it has grown: a command
  that prints `rea`, then `dy` and a newline, shows `rea`, then `ready`.
  Once the command ends, its last line counts as whole even without a
  newline.
- The handle serves `demi shell status`, `input` and `stop` while the
  command runs. A result that reports the command's end releases it;
  `demi shell output` goes on reading the output by its `commandId`.
- A result that reports a command's end attaches the media it viewed with
  `demi file view` ([Media the model views](#media-the-model-views)).

### Media the model views

The model looks at an image, a video or a PDF with `demi file view`, the one
command that shows the model a file. For example, it checks the sign-in page
and two frames of a recording in one call:

```text
$ demi browser screenshot t1 | demi file view
Screenshot of t1
Image: 1280 × 720 px, one pixel per CSS pixel
Viewport: 1280 × 720 CSS px, device pixel ratio 2, web
[image 1: image/png, 1280 × 720 px, 412000 bytes]
$ ffmpeg -loglevel error -i demo.mp4 -vf fps=1/5 frame-%02d.png && demi file view frame-01.png frame-02.png
[image 2: image/png, 1920 × 1080 px, 801233 bytes]
[image 3: image/png, 1920 × 1080 px, 799410 bytes]
```

The result shows this output and attaches the three images after it, in
that order; the page shows them under the call's row
([Media a tool returned](../product/file-previews.md#media-a-tool-returned)).
In this section the script that a `shell` call runs is the **job**, which the
`commandId` names. A **medium** is one image, video or PDF document that the
job showed the model.

#### What `demi file view` shows

- **The command hands the medium over, not its stdout.** `demi file view
  [<path>...]` reads each path, or its stdin when it names no path or names
  `-`, and gives each one to its job as one medium: in a pipeline, a command
  substitution, a loop or a background task alike. It writes nothing to its
  stdout, and the rest of the script never sees the bytes. Stdin that is the
  job's own input, which `demi shell input` feeds
  ([A job's own input](../execution/runner.md#a-jobs-own-input)),
  is not read: with no path, the command fails with
  `demi file view: no file named, and stdin is the job's input; name a file or pipe one in`.
- **Lines in the output.** The runner numbers the job's media from 1 in the
  order they reach it and writes each one's line into the job's output as it
  arrives, after the output before it: `[image 1: image/png, 1280 × 720 px,
  412000 bytes]`, `[video 2: video/mp4, 12.4 s, 1920 × 1080 px, 8598311
  bytes]` (the duration and size when the container's header gives them),
  `[document 3: report.pdf, application/pdf, 182044 bytes]`. The word is the
  medium's kind; the number is the job's. A document carries its file's
  name to the model, which a provider's document part takes as its title;
  one from stdin is named `document-3.pdf`. The handler of `demi file view`
  reads these facts from the medium's header, once, and they travel with the
  medium; the runner writes the line only after it has read what the job's
  output already holds, so a line never comes before output written before
  the command.
- **What it takes.** Bytes, at most 16 MiB, of a media type the conversation's
  model reads in a tool result: a type of the model-media table
  ([Accepted attachment types](../providers/models.md#accepted-attachment-types))
  that the model accepts and its provider carries in a tool result
  ([Media in tool results](../providers/providers.md#media-in-tool-results)).
  The job carries that list from its start, as the backend gives it for the
  node's model, with the model's name for the refusal line. A job that
  `demi host shell` starts on another Host carries none, since its output
  is relayed into the invoking command and no result reports its end: there
  the command fails with
  `demi file view: shot.png: a job on another Host shows the model nothing; pipe its bytes into demi file view in your own script`,
  as `demi host shell --host laptop 'cat shot.png' | demi file view` does. A path that fails gets a line on stderr; the command goes on
  with the next one and exits with status 1:
  - `demi file view: notes.txt: a text file; read it with cat notes.txt`
  - `demi file view: data.bin: not an image, a video or a PDF (48213 bytes)`
  - `demi file view: report.pdf: this conversation's model, deepseek-v4.1-flash, does not read application/pdf in a tool result`,
    or, when which types the model reads is unknown,
    `demi file view: shot.png: it is not known which files this conversation's model, deepseek-v4.1-flash, reads; its provider entry can name them`
  - `demi file view: capture.mov: 23.1 MiB; a medium is at most 16 MiB`
  - `demi file view: shot.png: No such file or directory`, the system's own
    words ([Handle an rpc call](../execution/commands.md#handle-an-rpc-call)).

  Stdin is named `stdin` in these lines.
- **Every other command writes bytes.** A program that makes an image writes
  its bytes as any program does, so `> shot.png` saves them and
  `| demi file view` shows them; `demi browser screenshot` does the same
  ([Images and large outputs](../browser/browser.md#images-and-large-outputs)).
  Bytes are binary when their first 8 KiB hold a NUL byte or they read as
  a media type Demi recognises, as git and GNU diff decide; any other bytes
  are text, and a byte that is no UTF-8 shows as U+FFFD, as a terminal
  shows it. One rule decides it for a job's stdout and for edit tracking's
  copies ([Edit tracking](../execution/edit-tracking.md)), so a stray byte,
  such as a raw line `grep` printed from a git object, costs one character
  rather than hiding the whole output, as it did in a product check. A
  job's stdout that is binary is never shown to the model: the output
  shows it as one line, `<binary stdout: 412000 bytes>`, and the result's
  line says what to do instead:
  `[binary stdout, 412000 bytes: not shown; to look at an image, a video or a PDF, pipe it into demi file view; to keep it, redirect it to a file]`.

| The script | What the model gets |
| --- | --- |
| `demi file view a.png b.png` | Two images, in that order |
| `demi browser screenshot t1 \| demi file view` | The screenshot, with its description from the screenshot's stderr |
| `demi browser screenshot t1 > shot.png` | Nothing of the image: `shot.png` holds the PNG |
| `demi browser screenshot t1 \| convert - -resize 50% png:- \| demi file view` | The half-size image alone |
| `for t in t1 t2 t3; do demi browser screenshot "$t" \| demi file view; done` | Three images, in the loop's order |
| `demi file view a.png & demi file view b.png & wait` | Two images, numbered in the order they reached the runner |
| `x=$(demi file view a.png)` | The image; `x` is empty |
| `cat shot.png` | The line `<binary stdout: 412000 bytes>` and the result's line saying to pipe it into `demi file view` |
| `demi host shell --host laptop 'cat shot.png' \| demi file view` | The laptop's image: the relayed bytes are this job's stdin to `demi file view` |

A medium goes to the model through the command that was asked to show it,
never through where its bytes happen to flow, so a file the script only
copies, pipes or saves never reaches the model, and one it views always
does, whatever the shell did with the command's stdout.

#### What a result attaches

Media reach the model only in the result that reports the job's end: the
`shell` result when the job ends within its window, otherwise its end
report ([Command reports](#command-reports)). A result that shows the job
running attaches none, so each medium is attached once and a job's media
arrive together, in their order. A `demi shell status` that shows the end
shows the media's lines and attaches nothing, since it is a job of its own:
the end report then still comes, carrying the media alone, though the look
showed the end.

The result attaches each of its media, in order, while these rules hold:

| Rule | Why |
| --- | --- |
| The model reads the medium's type in a tool result | A request carries nothing the model does not read; the model can change while the job runs |
| An image fits as [Images in the transcript](#images-in-the-transcript) fits it, and enters fitted; a video or a document is at most 16 MiB | Every request sends the same bytes, which every provider accepts |
| It is one of the result's first 20 attached media | One step's result stays a small part of a request: the fewest images a documented vendor takes in one request is 100 ([Request limits](../providers/models.md#request-limits)) |
| The base64 of the result's attached media, this one included, takes at most half of the model's request body limit ([Request limits](../providers/models.md#request-limits)) | A request still has room for the history around it. A medium that breaks this rule is skipped, and a later, smaller one may still be attached |

After the output, the result gives one line for each medium it did not
attach, and for each it attached in another form than it came, with what
happened; the original is the file the model viewed, so a line needs no way
to read it:

```text
[image 2: not attached: the model does not read image/webp]
[image 3: attached as image/jpeg of 2000 × 1000 px, fitted from image/png of 3000 × 1500 px]
[image 21: not attached: a result attaches at most 20 media; view it in another call]
[image 4: not attached: lost with the Host's connection]
```

#### Bounds and cut output

- A medium is at most 16 MiB, and a job keeps at most 32 media and 64 MiB of
  them. The runner keeps none beyond these: in that medium's place the
  job's output reads
  `[image 33: not kept: a job keeps at most 32 media and 64 MiB]`, and
  `demi file view` goes on. A job's media therefore take at most 64 MiB of
  its Host's disk.
- Media are kept apart from the output, so neither the output's 16 MiB bound
  nor a result's cut touches them. A build that prints 20 MiB of log and then
  views a screenshot attaches the screenshot, even when its line lies in the
  part of the output left out.

#### Where media are kept

The runner keeps a job's media in the job's directory as they arrive and
tells the backend of each one
([Pipes and output](../execution/runner.md#pipes-and-output)). When the job
ends, the backend reads each medium the result attaches, fits it, and stores
it as a blob in the conversation owner's namespace before the result reports
the end; the result holds it by reference, as every tool medium
([Media](#media)). A medium the result does not attach is not stored: the
model views its file again when it needs it.

### The whole output

A result shows the first and the last lines of a long output;
`demi shell output` reads all of it, a page at a time, the way a file is
read. For example, after the `npm test` above, the model finds the failures
and reads around one:

```text
$ demi shell output 17 --raw | grep -n 'FAIL'
2301:FAIL src/parse.test.ts
$ demi shell output 17 --lines 2295-2310
[command 17: lines 2295-2310 of 4720, stdout and stderr]
  2295	  ✓ parses an empty file (3 ms)
  ...
  2301	FAIL src/parse.test.ts
  ...
  2310	    at Object.<anonymous> (src/parse.test.ts:88:5)
```

`demi shell output <commandId>` is an `rpc` command
([Command declarations and execution](../execution/commands.md)): it runs in
the backend, from any shell of the conversation, the root's or a subagent's,
on any of the conversation's Hosts. Everything it prints is sized for the
model:

- **A page.** It prints numbered lines, as `cat -n` does: from the first line,
  or the lines `--lines <from>-<to>` names, as many whole lines as fit in
  12,000 characters, so its own result is never cut. A first line in brackets
  says what the page holds; when lines follow, a last line gives the command
  for the next page: `[next: demi shell output 17 --lines 2311-4720]`.
- **The newest lines.** `--tail <n>` prints the last `n` lines, or as many of
  them as fit; of a running command, the newest so far.
- **One stream.** `--stdout` or `--stderr` takes that stream alone, with line
  numbers of its own. Without either, the page holds both, merged in the
  order the runner read them, as the result showed them.
- **The bytes.** `--raw` prints the output as it is: unnumbered, unpaged,
  byte for byte, for pipes and files. Its lines are the pages' lines, so
  `grep -n` on it gives the numbers `--lines` takes, and
  `demi shell output 17 --raw --stdout > shot.png` saves a binary stdout. Printed into a result as it is, raw output is cut as
  any output is. Searching is the standard tools' work on `--raw`; the command
  has no search of its own.
- **A long line.** A page shows a line of more than 2,000 characters as its
  first 2,000 and a note after it:
  `[line 2301 is 48212 characters; whole: demi shell output 17 --raw | sed -n 2301p]`,
  so one line cannot fill a page.
- **Which commands.** Any command of the conversation, whichever agent ran it.
  A command of another conversation is unknown to it.
- **A command that runs.** It reads what the command's Host has kept so far,
  through the conversation's host access
  ([Host operations](../execution/sessions-and-targets.md#host-operations)).
- **A command that ended.** It reads the output the backend stored, which
  stays ([Retention](../backend/storage.md#retention)).
- **What is kept.** A command's output up to 16 MiB, all of it; beyond that,
  its first 8 MiB and its last 8 MiB. Lines are numbered as kept, and the
  merged pages show an unnumbered line where the rest was left out,
  `[... 734003200 bytes left out ...]`; `--raw` writes that line to stderr, so
  the bytes stay as they were. A command that prints without end, such as
  `yes`, therefore keeps at most 16 MiB, on its Host and in the backend alike.
- **A binary stdout** shows in the merged pages as the line the result shows,
  `<binary stdout: 412000 bytes>`; `--stdout` without `--raw` answers that the
  stream is binary and names `--raw`.
- **Failures** go to stderr with exit status 1:
  - `demi shell output: no command 17 in this conversation`;
  - `demi shell output: the output of 17 was not stored: <reason>`, when the
    backend could not store it;
  - `demi shell output: lines 5000-5100 are past the end: the output has 4720 lines`.
- **A reader that stops early**, such as `| head -n 20`, ends the command
  quietly ([Handle an rpc call](../execution/commands.md#handle-an-rpc-call)).

Where the output is kept:

- While the command runs, on its Host, in the job's directory, within the same
  16 MiB ([Pipes and output](../execution/runner.md#pipes-and-output)).
- When the command ends, the backend stores its output before any result
  reports the end. When each stream stayed within its first 8 KiB, which is
  most commands, the backend already holds the whole output, in the order the
  runner read it, and stores that; otherwise it reads the Host's kept output
  once, as it reads the command's edit copies
  ([Edit tracking](../execution/edit-tracking.md)). The output becomes a blob
  in the conversation owner's namespace, and the conversation records it by
  its command ([Command outputs](../backend/storage.md#command-outputs)). The
  Host then removes the job's directory. The result that reports the end is
  made from the stored output, so the lines it names are the pages' lines.
- A command that ended because its Host's connection was lost keeps what the
  backend received, followed by the line
  `[... 1048576 bytes lost with the Host's connection ...]` when there was
  more. So does a command whose kept output the backend could not read from
  its Host, with the line
  `[... 1048576 bytes not read from the Host: <reason> ...]`.
- The stored output is never part of a request, so storing it or removing it
  changes no request ([Prompt cache](../providers/providers.md#prompt-cache)).

### Live output

For example, the model runs `npm test` with a two-minute window while the
conversation is open in tabs A and B. Both tabs show the test output under the
call as the runner reads it. The window ends while the tests still run: the
call returns the command's handle to the model, and both tabs count the
command in the dock's Running chip, whose panel keeps showing its output as it
comes, although the model no longer looks at it. Tab C, opened now, shows the
end of the output at once and then follows as A and B do. When the tests end,
all three show the end.

```text
the job's stdout and stderr
  |
  v
runner: job_output, each with its stream and offset
  |       the first 8 KiB of each stream, always
  |       beyond them, while followed: the newest bytes,
  |       at most 16 KiB per stream every 250 ms;
  |       while not: the newest 8 KiB, at most every 2 s
  v
backend: the command's record
  |       the model's place and text
  |       the pages' view: its last 4,096 characters and their count
  v
the tree: shell_output to every attached page,
          new output at most every 250 ms, an end at once
```

- **The pages' view.** A command keeps one view of its output for the pages,
  apart from the model's place: its output in the order it reached the
  backend, with a note where the runner left some out. The pages are sent the
  view's last 4,096 characters (`tail`) and how many characters the view has
  held since the command started (`chars`); characters are Unicode scalar
  values. The view is the same for every page, whatever any page or the model
  read, and no read changes it. When the command ends, the view gets what the
  end adds: when the backend had not received all of the output, the whole
  output it stores ([The whole output](#the-whole-output)), whose end the
  pages then show anew; why the command could not run; or a binary stdout's
  description.
- **Which commands.** Every command of the tree, the root's and each live
  subagent's, several at once, on a Cloud and on a paired device alike: both
  are Hosts behind a runner.
- **While a page is attached.** While at least one connection is attached to
  the tree ([Connections and the live tree](#connections-and-the-live-tree)),
  the backend follows every running command of the tree: the runner then sends
  each stream's output beyond its first 8 KiB as it comes. A command's start and
  every change of its view reach every attached connection as `shell_output`:
  the start and new output at most every 250 ms, the tree's changed commands
  together, and the command's end at once. Output that comes after a quiet
  quarter of a second goes at once. A command therefore sends a page at most
  four frames a second however much it prints, and cannot fill an outbox
  ([Order and delivery](#order-and-delivery)) by itself.
- **A page that attaches** receives each live command's view in its
  handshake, then every change as the other pages do. A live command is one
  that runs in a node's shells, or that the node's transcript last shows
  running and its shells still hold. When no page was attached before, the
  backend was not following, so the handshake shows what the backend holds;
  the runner's newest output follows as soon as the runner starts following.
- **No page attached.** When the last connection detaches, the backend stops
  following and sends nothing; the runner sends each stream's first 8 KiB
  again and, beyond them, the stream's newest 8 KiB every 2 seconds, which the
  model's view shows and its idle time counts from
  ([Results and previews](#results-and-previews)). The next
  connection to attach starts from its handshake.
- **The end.** A command's last frame shows its end: its exit, a stop (a
  page's stop, `demi shell stop`, a Stop of the action that
  started it), or the end of its node's shells: the abort or close of its
  subagent, or the tree's interruption for a Host transition. The tree's
  disposal ends no command: its commands run on and report when the tree is
  live again ([Dispose and restore](#dispose-and-restore)). No frame of the
  command follows its end.

What keeps the output coming, and where each part is released:

| Part | Lives in | Released |
| --- | --- | --- |
| The runner's messages beyond a stream's first 8 KiB, with their timer | The job's task on the runner | With the job; the timer runs only while a message waits |
| The backend's following of a job: it watches whether a page is attached and tells the runner | The job's task in the node's shell environment | With the job |
| The commands whose new output is not sent yet | The tree | Emptied when sent, when a command ends, and when the last connection detaches |
| The task that sends them | The tree; it has a timer only while commands wait | With the tree |
| The reports of new output and ends | The tree's feed, which each shell environment of each node holds without keeping the tree alive | With the environment |

### Dispatch and failures

- Before the first tool of a round runs, the session saves its checkpoint, so
  every call it is about to run is stored as executing. A process that dies
  during a tool leaves the call executing, and restore completes it as
  interrupted without running it again ([Dispose and restore](#dispose-and-restore)).
- A round's calls run in steps, in the order the model requested them:
  consecutive `shell` calls are one step and start together; each other
  call is a step of its own. A step ends when each of its calls has returned,
  and the next starts then. Each result is recorded as its call returns, in
  its call's block, so the page shows a call's end and its changed files
  while the step's other calls run; the model still reads the results in the
  order of the calls. Waiting input is written after each step
  ([Input](#input)). Since every call is `shell`, consecutive calls are one
  step: a response's calls all start together.
- Each `shell` call of a step is a job of its own
  ([Running shell tools](#running-shell-tools)), so the calls of a step never
  wait for one another.
- A Stop during a step ends the calls still running; a call that had
  already returned keeps its result.
- Each call takes its own lease of the conversation's file gate, so a step's
  calls never take a lease while holding another
  ([Host operations](../execution/sessions-and-targets.md#host-operations)).
- The model is told that the commands of one response run at the same time,
  so commands that depend on each other go in one script, joined with `&&`,
  or in separate responses. Models already treat the calls of one response
  as independent, and harnesses such as Claude Code run them together.
- A tool that fails completes its call as an error `Tool failed: <message>`.
- An action that fails before its calls ran, such as when the save before
  dispatch fails, completes them as aborted, with that failure as the
  reason, as a stop does, so the next request replays no call without a
  result.
- A tool never reaches into its session. It returns its result.

### Interrupted calls

For example, the backend restarts for a configuration change while the
model watches command 254, a test suite. The model's next request shows the
call ended:

```text
Tool call interrupted: the backend shut down while this call watched command 254, which keeps running; look at it with demi shell status 254.
```

A call that something other than its tool ends completes as an error that
says what ended it and what became of its command, since a model told only
`Tool call aborted: shell_exec` spent calls on `ps` and `journalctl` to find
out:

| What ended the call | Its result |
| --- | --- |
| The user's Stop | `Tool call aborted: the user stopped the turn; command 254 was stopped.` |
| A hold of the conversation, such as a Cloud reset or a deletion | `Tool call aborted: <the hold, such as "the Cloud is being reset">; command 254 was stopped.` |
| The backend's shutdown | `Tool call interrupted: the backend shut down while this call watched command 254, which keeps running; look at it with demi shell status 254.` |
| The backend's crash, found at restore | `Tool call interrupted: the backend stopped before its result was recorded; command 254 keeps running if its Host kept it.` |
| An action that failed before its calls ran | `Tool call aborted: <the failure>.` |

A call that watched no command ends the line after the reason. A call that
watched one follows the line with the command's output since the model's
last look, as a result shows it, so the model sees how far the command got
without looking again: after a Stop in a product check, the model's next
step was `demi shell status` only to read `ok group 1`. A command an
interrupted call leaves running, after a shutdown or a crash, reports its end
as any command whose call returned does ([Command reports](#command-reports)).

### Lost commands

A command whose runner lost it, which the backend learns when the runner
connects again ([Recovery and persistence](../execution/sessions-and-targets.md#recovery-and-persistence)),
ends as lost with its reason, and the node that ran it is told as of any
other end ([Command reports](#command-reports)):
`Command 437 (bun run dev) was lost: Demi was upgraded and the Host's runner
replaced itself. Start it again if it is still needed.` A conversation
browser tab that its browser lost reaches the node the same way:
`Tab t1 was closed: the browser ended with the Host's runner.` The command's
handle answers `demi shell status` and `demi shell output` with the same reason.
Before, a dev server lost with a two-second connection drop went unnoticed
for three hours, until the agent's next `demi shell stop` said it had
already ended.

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
| A command's medium | `2` | Its command | The runner, as the medium reaches it ([Media the model views](#media-the-model-views)) |
| An agent | `0` for the root, then `1`, `2`, … in spawn order | The conversation | The backend, when the agent is spawned ([Model-facing surface](subagents.md#model-facing-surface)) |
| An agent's round | `1` for its first run, one more at each resume | The agent | The agent's supervisor |
| A conversation browser tab | `t7` | The conversation | The backend, when the tab is registered ([One tab registry](../browser/browser.md#one-tab-registry)) |
| An attachment the agent uploaded | `a3` | The conversation | The backend, when `demi attachment upload` stores it ([Attachment commands](../execution/commands.md#attachment-commands)) |
| An element reference | `e37` | Its tab | The conversation browser, as it observes the tab |
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
  ([Attachments](../product/product.md#attachments)).
- **Inside Demi**, an identity keeps the form its storage and its wire need;
  this rule is about the text the model reads and writes.

## Transcript

Words used for session data:

| Word | Meaning |
| --- | --- |
| transcript | A session's history: its ordered blocks |
| status | A point-in-time answer an operation returns, such as a command's status; never stored as history |
| checkpoint | The durable, restorable state of one session ([Tree store](#tree-store)) |
| whole output | A command's output as kept: on its Host while it runs, in the backend once it ends ([The whole output](#the-whole-output)) |
| view | Bounded data a block carries for the user; never replayed to the model |
| blob | Content-addressed bytes, such as media, edit copies and commands' outputs, in the conversation owner's blob namespace |

### Block types

| Block | Written by | The model receives | The user sees it |
| --- | --- | --- | --- |
| `user` | A send or an edit: the submitted content and, for a subagent, its identity (`preamble`, [Child context](subagents.md#child-context)) | A user message: the preamble, then the content | Yes; the only editable block ([Message editing](message-editing.md)) |
| `context` | The session before a provider request, with the text one context source answered ([Context](#context)), the source's name (`source`), and for the instructions source the list of what the text holds (`instructions`, [What the card lists](instructions.md#what-the-card-lists)) | A user message with its text | No |
| `wakeup` | Command reports that arrived together, with the placement `new_turn` or `steer`: each report's command, its call's title and what it reports | The reports' text, as a user message or as a steer | As a report row ([Command reports](#command-reports)) |
| `steer` | A human steer, at a continuation boundary | A steer in the current turn | Yes |
| `agent_message` | Another agent of the tree ([Communication](subagents.md#communication)) | A steer holding the message's source envelope | As a receipt row |
| `resume` | A turn continuing after a cut: `resume`, compaction inside a turn, or a model switch that landed inside a turn and compacted | A user message: "Continue from where you left off." | No |
| `abort` | Stop | Nothing | Yes, until the turn is continued and `isResumed` is set |
| `thinking` | The provider: reasoning text and its signature | The text; a signed block whole | Yes |
| `redacted_thinking` | The provider: opaque reasoning data | The data, whole | No |
| `text` | The provider: assistant text, marked `forkable` once complete ([Eligibility](conversation-fork.md#eligibility)) | The text | Yes |
| `tool_call` | The provider's call, completed by the session with the result | The call and, once completed, its result | Yes |
| `response` | The provider: the usage of one completed request | Nothing; its usage anchors the context estimate | No |
| `error` | A failed request or an interrupted turn ([The failure record](failures-and-recovery.md#the-failure-record)); `outsideTurn` when the failure ended no turn ([The unfinished turn](failures-and-recovery.md#the-unfinished-turn)) | Nothing | Yes |
| `compaction_boundary` | Compaction: the summary, inserted where the kept history begins | A user message: "Previous conversation summary:" and the summary | Through its marker; at its own place only when an edit removed the marker |
| `compaction_marker` | Compaction: the estimated size of what was summarized, appended at the end | Nothing | Yes, as the compaction's divider, with its boundary's summary size |

Each block carries the time it began by the backend's clock: a `text` block
the arrival of its first text, a tool block the call's start. The page shows a
reply's time from it, and never shows a past time as a future one.

A compaction shows where it was triggered, not where its summary sits. For
example, the user compacts after the eleventh message: the boundary goes in
before the tenth answer, and the marker after the eleventh message, so the
divider reads below the eleventh message, where the user acted. While the
phase is `compacting`, the page shows the divider in progress at the end of
the transcript, before pending steers and queued messages, which is where the
marker will be appended; the finished divider then takes its place. A pass
that fails ends with its `error` block at the same place
([Retries](failures-and-recovery.md#retries)). A compaction is not the end of
a turn: behind its divider, the turn before keeps its Resume or Continue
([The unfinished turn](failures-and-recovery.md#the-unfinished-turn)).

A `user`, `context` or `wakeup` block with the placement `new_turn` opens an
input turn: recovery treats it as the start of its turn
([Recovery](failures-and-recovery.md#recovery-is-one-mechanism)). For retry,
the first `agent_message` of a continuation opens its turn as well. The other
blocks belong to the turn they appear in.

A `tool_call` block holds the provider's `toolUseId` and `toolName`, the call's
`input` as the JSON text the provider supplied, its `status` (`executing`,
`completed` or `error`), its `output`, and its `view`.

### Context

What the model must learn between its requests, and what differs by user or
conversation, reaches it as `context` blocks, never through the system prompt.
For example, the user switches the conversation from the Cloud to their
laptop. Before the node's next request, the execution context source answers
the new target, and the model receives it as a user message.

Before each provider request of a node, the session asks every context source
the product supplies, in the product's order:

- Each source is given the text of its own `context` blocks that the model
  receives, those from the last `compaction_boundary` on, oldest first, with
  the node's working directory and the id of its current input turn, and
  answers new text or nothing. After a compaction, a source therefore tells
  the model again what the summary may have left out.
- The product's order is the execution context, then the instructions, then
  each plugin that is a context source.
- Each answer becomes one `context` block that names its source: `execution`
  for the conversation's execution context
  ([Switch the primary target](../execution/sessions-and-targets.md#switch-the-primary-target)),
  `instructions` for the user's personal instructions and the project's
  instruction files ([Instructions](instructions.md)), which also lists what
  its text holds, or the id of the plugin that answered
  ([Prompt text and context](../architecture/plugins.md#prompt-text-and-context)).
- The blocks are appended before the request and saved at once, so a request
  never carries a context its transcript does not hold.
- A source that fails adds no block, and the failure is logged. Nothing is
  lost by it: the source is asked again before the next request.

### Replay

The model receives the blocks from the last `compaction_boundary` onward, the
replayed blocks, each as the table says. A long text is cut in the middle by
the replay bound ([Text bounds](compaction.md#text-bounds)). Signed thinking
and redacted data are replayed whole, because the vendor verifies them as they
were sent. Each medium is replayed with the bytes the session holds for it,
and a tool's medium that is gone as its text ([Media](#media)). A message's
reference is replayed as its text, and an attachment record
([Attachments](../product/product.md#attachments)) as a tag that names the
file, its media type, its size and its path, never its content. A tool
call's input is replayed as the JSON value the provider supplied, or as text
when it is not valid JSON. A call of a tool the request no longer declares,
such as `yield` or `shell_status` in a transcript from before they were
removed, is replayed with its result as the model's text, `[called yield {"durationMs":600000}: yield
scheduled]`, since a vendor may refuse a call of a tool it was not given.

An image, video or document is replayed as a text that names it and says why
it was not sent when the request's model does not accept its type
([Accepted attachment types](../providers/models.md#accepted-attachment-types)),
or when its base64 is longer than half of the model's request body limit
([Request limits](../providers/models.md#request-limits)), which leaves no room
for the history around it:

| The medium | Its text |
| --- | --- |
| Of a type the model does not accept | `[<kind>:<name>, not sent: the model does not accept it]` |
| Over half of the model's body limit | `[<kind>:<name>, not sent: too large for the model's requests]` |

The kind is `image`, `video` or `document`, and the name is the media type of
an image or a video and the file name of a document. The text depends only on
the medium and the request's model, so every request to that model carries the
same text. For example, after a switch to a model without video, a tool
result's video reaches that model as
`[video:video/mp4, not sent: the model does not accept it]` in every request,
and a switch back sends the video again.
The token estimates for a model weigh such a medium as that text
([Block estimates](compaction.md#block-estimates)).

Reasoning between the last `compaction_boundary` and its marker, which
compaction kept after the summary, is marked as kept past a summary: the
history it followed is gone, and a provider whose vendor checks reasoning
against that history leaves it out
([Per vendor](../providers/providers.md#per-vendor)). When an edit removed the
marker, all reasoning after the boundary is marked, since leaving reasoning
out is safe and replaying reasoning whose history changed is not.

Replay reads only what the blocks hold for the model, and which media the
request's model accepts, never an ID or a time, and a block keeps its place.
The same history therefore gives the same items for the same model in any
session. Each request's items begin with the previous request's: new blocks
are appended, a tool call's result completes a call no request has carried
yet, and a view is never replayed. Only compaction, a model switch and an edit
change what an earlier request carried; a retry and a resume cut back to an
earlier request. This is how a session keeps its vendor's cache
([Prompt cache](../providers/providers.md#prompt-cache)).

A request also says how many of its leading items the session's latest
answered request carried. That request is the one whose `response` block
comes last after the last compaction, that is, after the last
`compaction_boundary` and its marker when the history has them; its
answer begins at the first of the thinking, text and tool-call blocks directly
before that `response` block, and its content is what the blocks before its
answer replay. Compaction summarizes that content
([One pass](compaction.md#one-pass)), and a vendor reads its cache entry at its
end. Before any request is answered after the last compaction, the request
says none. With Claude Code, a run that ends in a batch of tool calls
([Tool-call batches](../providers/claude-code.md#tool-call-batches)) writes no
`response` block, because the CLI reports usage only on the line that ends its
turn
([Requests over stream-json](../providers/claude-code.md#requests-over-stream-json)),
so its latest answered request is the last request that ended a turn. A title request says instead that it is no session's
([Conversation titles](../product/product.md#conversation-titles)).

### Media

A message's images, videos and documents and a tool result's images,
videos and documents are stored once, as blobs in the conversation owner's blob namespace,
and blocks hold them by reference: `{ type: "ref", ref, mediaType }`, with an
image's `width` and `height` in pixels, which its fitting reads, a video's
when the backend reads them from its container's header, and a
document's `fileName`. The session keeps a medium's bytes only while a
provider request can still send them.

For example, the agent runs a command that prints a 400 KB screenshot:

```text
the tool returns the PNG's bytes
  -> the session's store puts them under their SHA-256:  blobs/<owner>/<sha256>
  -> the tool_call block's result holds the reference, and the
     session holds the bytes beside the transcript
saves, patches, resets, syncs, other pages, edits, Forks:  the reference
each provider request while the block is replayed:         the held bytes
```

A medium has one form in each place, and each form is a type of its own, so
a block cannot hold bytes and a provider cannot receive a reference:

| Where | The medium |
| --- | --- |
| What a tool returns | Its bytes and media type |
| Blocks, queued messages, pending steers, checkpoint rows and frames | A reference; a message's image or video may name a URL the vendor fetches instead |
| A provider request's items | Its bytes and media type, or that URL |

- **Storing.** A medium's bytes are stored when the medium enters, and never
  again; an image is fitted before that
  ([Images in the transcript](#images-in-the-transcript)). An upload is
  stored when the page uploads it, and the medium a message receives from it
  references the upload's own blob, or the fitted image's, which the backend
  stores as it resolves the message's content
  ([Media by reference](../backend/backend.md#media-by-reference)). The
  session has its store put a tool's medium after the tool returns and
  before the result enters the transcript. The put computes the bytes'
  SHA-256, which names the blob, and sends no bytes for a blob the namespace
  holds already ([The object store](../backend/storage.md#the-object-store)).
  A tool's medium whose put fails is gone from its result (below), so the
  turn goes on and no block names a blob that was not stored.
- **A medium that is gone.** A tool result's medium whose bytes
  could not be stored gives way, in its place, to a part of its own that says
  what it was and why it is gone:
  `{ type: "gone", kind, mediaType, cause }`, where `kind` is `image`,
  `video` or `document`, and `cause` is `{ type: "not_stored", error }`, with the store's
  error. The model reads the part as one line of text, which the agent
  renders in one place: `[<kind> not stored: <reason>]`. Nothing that was
  stored is ever removed ([Retention](../backend/storage.md#retention)). The
  page shows the part where the medium was
  ([Media a tool returned](../product/file-previews.md#media-a-tool-returned)).
  A message's media are never gone: an upload is stored before a message can
  name it.
- **References.** The transcript, the queued messages, the pending steers,
  every checkpoint row and every frame hold media only by reference, or by
  the URL a message's image or video names: their types have no place for
  bytes. Nothing converts media when a block is saved or sent, and an edit or
  a Fork copies references.
- **Held bytes.** The session holds something only for the media that its
  replayed blocks or its waiting input (queued messages and pending steers)
  reference: the medium's bytes, or the fact that its blob is missing. A
  medium arrives with its bytes: a tool's medium with its own, an upload's
  with those the backend read to write the file to the Host or with the
  fitted image's. When a new
  `compaction_boundary`, a history rewrite or a withdrawn input leaves a
  medium unreferenced, the session lets go of it before its next request. A
  live tree therefore holds, per live session, at most the media its next
  request sends and those of its waiting input.
- **The model's view.** Replay, the token estimates and compaction read the
  replayed blocks together with what the session holds for each medium they
  reference: its bytes, or the fact that its blob is missing
  ([Replay](#replay), [Token estimates](compaction.md#token-estimates)).
  Before the session builds that view, it reads the blob of each replayed
  medium it holds nothing for, which after a restore is every one, a few at a
  time concurrently, since on S3 each read is a round trip; it builds the
  view only once it holds something for every replayed medium. Opening a
  conversation therefore reads no blob, a request never reads a blob the
  session holds, and no blob from before the last `compaction_boundary` is
  read.
- **Requests.** Replay puts each medium's held bytes into the request's
  items, and a medium whose blob is missing becomes the text
  `[missing <kind>]` there, so the turn goes on; the transcript
  keeps the reference. A provider request is the only place a medium carries
  its bytes, and it carries no reference, so a provider never has one to
  refuse.
- **Stable requests.** Within a live tree, a medium reaches the model in one
  form for as long as it is replayed: its held bytes, or the missing text for
  a blob found missing. Bytes the session let go of and reads again are the
  same bytes, since a blob's name is their hash and no blob is ever deleted
  ([Retention](../backend/storage.md#retention)). Media therefore never
  change the start a request shares with the previous one.
- **Failures.** A read that fails, rather than finding the blob missing,
  fails the action before its request, as a failed save does; the next
  action reads again.
- **Where the bytes go.** The agent defines these rules, and the backend
  decides where the bytes go. A session reaches the owner's blob namespace
  only through its tree store ([Tree store](#tree-store)); the agent's server
  has no blob store of its own.

### Images in the transcript

An image enters the transcript once: when a tool result attaches it
([Media the model views](#media-the-model-views)), or when a message's upload
becomes its native medium
([Attachments](../product/product.md#attachments)). It is fitted then to what
every provider accepts of one image, at most 2,000 px on each side and
5,000,000 bytes as base64, which is 3,750,000 bytes of image, and it never
changes afterwards, so every request sends the same bytes.

| The image | What enters the transcript |
| --- | --- |
| PNG, JPEG or WebP, at most 2,000 px on each side and 3,750,000 bytes | The image, unchanged |
| Larger than 2,000 px on a side | The image turned upright by its EXIF orientation and scaled down to fit 2,000 × 2,000 px, keeping its aspect ratio: a JPEG as JPEG at quality 90, a PNG or WebP as PNG |
| A GIF | Its first frame as PNG, fitted the same way |
| Over 3,750,000 bytes after that | The fitted image as JPEG at quality 85 |
| Still over 3,750,000 bytes, not decodable, or needing more than 256 MiB to decode | No image: a tool result says why, and an upload stays an attachment the model reads by path |

For example, a screenshot of 3,000 × 1,500 px enters as 2,000 × 1,000 px. The
limits follow the vendors: the Anthropic API refuses a request with more than
20 images when a side of any of them exceeds 2,000 px, and OpenAI and Gemini
scale a larger image down themselves; the vendors' strictest limit for one
image is that of the Anthropic models on Amazon Bedrock and Google Cloud, 5 MB
as base64. A GIF becomes a PNG because Gemini does not read GIF and the other
vendors read only its first frame. A fitted image carries no EXIF data, so its
orientation is applied before it is scaled.

The original stays where it came from: an image the model viewed in the
file it viewed, and an upload in its attachment file
on the Host and in its upload blob, whose attachment record keeps the
original's size and hash. Fitting decodes and encodes on the
blocking pool ([Blocking work](../architecture/concurrency.md#blocking-work)).
Videos and documents are not fitted; they count toward the request's size
([Request size](compaction.md#request-size)).

### Views

A `tool_call` block's `view` carries bounded data for the user. It is never
replayed to the model, it never embeds unbounded payloads such as full output,
file bodies or raw bytes, and its type is fixed per tool by `kind`:

| `kind` | Fields |
| --- | --- |
| `shell` | `status` (`running`, `exited` or `aborted`); `commandId`; `exitCode`, once exited; `runningMs`; `idleMs`; `chunks`, the last 32,768 characters of the output the result covers, stdout and stderr merged, each chunk tagged with its stream, a line that stands for bytes the output does not hold tagged as stderr; `viewTruncated`, true when that window or the output itself was cut; `files` and `filesTruncated`, once the command has exited and changed files |
| `repeated_shell` | `script`, `count` |

The shell view's characters are Unicode scalar values, counted from the end so
the newest output stays. `files` lists one entry per changed path with its line
counts and, for each edit segment, the blobs of its two sides when they were
stored ([Edit copies](../execution/edit-tracking.md#edit-copies)).
The model's result and the view come from the same command status; the view
shows nothing the model could not read from the command, except `files`, which
exists only for the user.

### Patches and versions

Every change a session makes to its transcript is recorded as patches, each
naming the index of the block it touches:

| `op` | Fields | Meaning |
| --- | --- | --- |
| `add` | `index`, `value` | Insert a block at the index |
| `replace_block` | `index`, `value` | Replace the block at the index |
| `append_text` | `index`, `delta` | Append text to the block at the index |
| `truncate` | `length` | Remove every block from the index `length` on |

Consecutive appends to one block merge into one `append_text`. A rewrite of
history, such as a retry, a resume's unwind or an accepted edit, is one batch:
a `truncate` at the first block it changes, then an `add` for each block
from there. It never sends the blocks before the cut, which may be most of
a long transcript, and which a page may not hold
([Transcript windows](../product/web-application.md#transcript-windows)).
Each batch of patches advances the transcript's revision by one.

A transcript version is `{ epoch, revision }`. The epoch is new each time the
session's transcript is built, at creation and at every restore, so a version
taken before a backend restart never matches after it. Message editing uses
the version to refuse an edit made against a transcript that has changed since.

## Rendering boundary

The transcript and the frames already describe tool calls completely.
Renderers read them directly; there is no separate render model.

1. A `tool_call` block is the stored envelope of a call, not a render type.
2. A renderer dispatches on the block's `type` first and, for `tool_call`, on
   its `toolName`.
3. The `shell` tool has its own rendering; it never falls through to the
   generic tool card. A call to a tool the runtime no longer has, such as
   `shell_status`, `yield`, `shell_write` or `shell_abort` in a transcript
   from before they were removed, renders as the generic card. A stored
   `shell_exec` call became a `shell` call when its conversation's database
   was migrated, since `shell` is what it became
   ([Schemas and migrations](../backend/storage.md#schemas-and-migrations)).
   A stored wakeup of a yield became what it stood for: one a command's end
   fired, that command's report, titled by its call's `description`; one its
   time fired, a `resume` block, since all it told the model was to go on.
4. The generic tool card is only for a tool name the runtime does not have. A
   model can request one; its call then ends with `Tool not found`.
5. The renderers live in `web-ui`. `web` and `web-gallery` feed them the same
   blocks and frames, and neither defines a second data model.

A renderer uses the `tool_call` fields as [Transcript](#transcript) defines
them: `toolName` selects the rendering, `input` is JSON text the renderer
parses, `status` and `output` give the result, and `view` enriches the display
with the command's own output and status.

A block the page read from a page of the transcript is in its light form,
without what only its open row shows ([Light form](../product/web-api.md#light-form)).
Its folded row renders from what it carries, the same as the block whole;
opening the row reads the block whole and shows it in place, with the
row's loading state until it arrives
([Transcript windows](../product/web-application.md#transcript-windows)).

The media a result carries are parts of its `output`, held by
reference ([Media](#media)); the view never holds them. Each tool's rendering
shows them with the call, and a medium that is gone where it was
([Media a tool returned](../product/file-previews.md#media-a-tool-returned)).

Live frames add to the transcript; they do not replace it:

- `transcript_reset` and `transcript_patch` are the primary input.
- `shell_output` adds a command's live output and status
  ([Live output](#live-output)). While the `shell` call that started the
  command runs, the page shows them under that call, which the frame names
  (`toolUseId`, in the subagent's transcript when `subagentId` is set). Once
  the call has returned, a command that still runs is one of the
  conversation's running commands: the dock's Running chip counts it, its
  panel shows it in a tab titled by the call's `description` (cut at the
  end, whole in its tooltip), and its output keeps coming there, while
  the call keeps the view its result stored. The panel's terminal opens with
  the script as a terminal shows what was typed: `$ ` and the first line, a
  continuation line `> ` for each further line, wrapped at the terminal's
  width, the prompt muted and the script in the emphasized text color, then
  the output; the line is the page's, not part of the command's output. A page adds to what it shows
  only the characters beyond those it has shown (`chars`), so a terminal
  keeps its scrollback; after a gap it shows the `tail` anew. A command's tab
  shows its status as its last frame, or after a reload its stored view,
  gives it: running, exited, or stopped (`aborted`). A command the user or
  the agent stopped did not finish, so it is never marked done.
- `shell_write_result` and `abort_result` acknowledge the user's controls.
  They do not mean the command or the turn has finished, and they do not
  replace the `tool_call` rendering.

A patch replaces a block at its index, and renderers key blocks by their id,
so an update never shows a second record. Renderers title a `shell` call by its
`description`, which the tool requires; a title too long for its row is cut
at the end with an ellipsis. A call whose script only looks at commands with
`demi shell status` counts in its work group as a check
([Work groups](#work-groups)). A call of a tool the request no longer
declares, such as a stored `yield` or `shell_status`, shows as the generic
tool card. Every shell row, a run or a look,
marks how its command ended the same way, and only when something went
wrong: a small tag after its title, the gallery's `Tag`, red *Failed* for an
exit code other than 0, grey *Stopped* for a stopped command, with the exit
code in the tag's tooltip, *Exit code 1*; one that succeeded shows nothing,
and one whose command runs shimmers, as long as it runs: also after its call
returned and the command runs on as one of the conversation's running
commands, until its end arrives. The expanded row says the end in words above the
output. Component structure, expansion, icons, typography, motion and the
presentation of changed files are shown in the gallery, not here.

### Work groups

An agent often takes many steps in a row: it thinks, runs a command, thinks
briefly, runs three more. Listed one per row, such a run pushes the answer
off the screen. The transcript shows each run of consecutive steps as one
row, as Cursor and Claude's own apps do. For example, the user asks to fix
failing tests, and the agent checks a directory, runs the tests, sets a
script's execute bit, patches a status file and runs the suite again, with
thinking between. While it works, one row shows the newest step, each new one
rolling over the one before; once the answer comes, the row reads *Ran 5
commands*, and a click opens it to every step.

- A step is a `thinking` block, a `tool_call` block or a call being written
  ([Calls being written](#calls-being-written)). A run is the steps between
  two other blocks, such as the user's message and the answer's text.
- Thinking without text shows only while it is the newest block of a
  running turn, as what the agent does now; once anything follows it or the
  turn ends, it has nothing to show and goes, from the transcript and from
  the steps of an opened row.
- The run at the end of a running turn is one row. While it holds a lone
  call, that row is the call's own, as an ended lone call is, and opening it
  shows the call, not a stack of one step. From its second step on, the row
  is the run's: folded, it is its newest step, shimmering while it runs, a
  step that arrives replacing the one before, a call being written rolling
  over to its description once written, and the call's block, which then
  takes its place, keeping that face. Opened, the row stands still as a
  stack and what runs, *Running 4 commands*, with the steps under it;
  opening and folding change the row at once rather than rolling it. The
  run's first step rolls into the tail row first, as any block does, and a
  run's row is the turn's activity, so no Requesting row shows under it; a
  lone call that ended is not, and the turn's Requesting shows under it
  until the next step comes.
- An ended run stays one row when it holds a call and another step, and a
  lone thinking stays its own row; a lone call is its own row. The row reads
  what the run did, each kind of call once, in the order the run first did
  it: *Ran 5 commands* for `shell` calls, *checked 2 commands* for calls
  whose script only runs `demi shell status`, counting the commands they
  looked at; a tool of
  another kind reads *used* and its name. For example, a run that thinks
  and checks command 17 twice reads *Checked 1 command*. The row never reads a bare count of steps,
  which says nothing of what happened. A failed command shows its tag on its
  own row inside, not on the group's.
- Folded, the files the run's calls changed show under the row, each once,
  counted from its two ends within the run ([What the conversation
  shows](../execution/edit-tracking.md#what-the-conversation-shows)); open,
  each call shows its own.
- A reader who opens or folds a row keeps it where it is: the rows after it
  move, and the transcript neither follows its end nor corrects for the new
  height until the fold has ended.

### Calls being written

A model writes a call's input before Demi can run it, and a command with a
long script takes seconds to write. For example, the model writes "Let me
write a categorizer." and then a `shell` call whose script is a 200-line
file. For the 14 seconds the model spends writing the script, the page shows
a shimmering *Write the categorizer* row under that sentence; when the call
is whole, the row becomes the call's block, and the command runs.

- The session keeps, live only, the calls of the running request that the
  provider has opened (Tool call start, [A run](../providers/providers.md#a-run))
  and not yet handed over whole. Each has its tool-use ID, its tool's name
  and its `description` once the model has written that string whole; the
  session reads it from the input written so far with a parser that accepts
  an unfinished JSON document. Nothing of a call being written is stored,
  replayed to the model or saved in the checkpoint: until it is whole, it is
  not a call.
- The server sends `pending_calls` with the complete list whenever it changes:
  a call opens, its description is read, or it leaves. A call leaves the list
  in the same step that adds its `tool_call` block, so a page never shows the
  call twice or not at all; the request's end, failure or cancellation empties
  the list. A page that opens the conversation receives the list after
  `pending_steers`.
- The page shows each call as the newest step of the run at the transcript's
  end ([Work groups](#work-groups)), its tool's row, shimmering, titled by its
  `description`. Until that is written the
  title is *Preparing a command…*. A page drops a
  pending call whose `toolUseId` it already holds as a block.
- A call's start completes the text before it
  ([Eligibility](conversation-fork.md#eligibility)).

A vendor that sends calls only whole has no calls being written: between its
text and its call, the tail row says Requesting
([Recovering an unfinished turn](../product/product.md#recovering-an-unfinished-turn)).

### Tool descriptions

Every tool's input takes a `description`: a short title, shown to
the user, for what the step does, as a command in the imperative: `Install
Chrome for Testing`, `Start the browser with a blank tab`, `Run the type
checker`. The model writes it before the step runs, so it never states a
result: a title such as `Chrome for Testing installed` would claim success
while the download still runs, and still claim it after the step failed. The
block shows whether the step runs, ended or failed beside the title. The title
names the work the user cares about, not how the tool works.

1. `shell` requires a non-empty `description`: without a title the block
   would show the raw script, which tells the user little. A call without
   one is refused as invalid input, so the model adds the title and calls
   again.
2. `description` affects display only. It changes neither the shell's
   behavior, the tool result, nor what the model receives on replay.
3. A `description` does not state a result or a state reached, does not
   describe waiting, pausing or tool mechanics, is not a generic action or a
   bare noun, and does not hold scripts, output, protocol state, step numbers,
   tool names, ids, internal labels or reasons.

## Frame protocol

A page of the web app opens `WS /api/conversations/:id/stream` for a
conversation ([Web API](../product/web-api.md)), with the session cookie
([Authentication and ownership](../backend/backend.md#authentication-and-ownership)).
The connection belongs to that one
conversation: the backend supplies the session id and the working directory
from the conversation's target, and the client never sends them. The types of
every frame are Rust types, and the web app validates frames with the schemas
generated from them ([Generated TypeScript](../architecture/contracts.md#generated-typescript)).
`ConversationClient`, in `@demicodes/conversation-client`, is the web app's client of this
protocol.

For example, a page opens a conversation whose root is running. It has read
the latest page of the transcript, blocks 2,940 to 2,999
([Pages](../product/web-api.md#pages)), and asks the stream for the blocks
from its last one on:

```text
client                               server
open { from: 2999, edge: b_2999 } -> attach to the live tree
                <------------------- opened
                <------------------- transcript_reset { start: 2999, blocks, version: { epoch, revision: r } }
                <------------------- phase, queue, pending_steers, pending_calls
                <------------------- subagent started + subagent_transcript_reset,
                                     for each live subagent, depth first
                <------------------- shell_output, for each live command
                <------------------- transcript_patch { revision: r + 1 }
```

### Client frames

| Frame | Meaning | Answer |
| --- | --- | --- |
| `open { from, edge }` | Attach this connection to the conversation's tree, restoring the tree when it is not live, with the model selection the conversation's record holds; `from` and `edge` name the root's blocks the page holds ([Where a reset starts](#where-a-reset-starts)) | `opened`, then the snapshot frames |
| `send { messageId, content }` | Submit a message ([Input](#input)) | Transcript, phase and queue frames |
| `edit_and_send { request }` | Replace a user message and its suffix ([Message editing](message-editing.md)) | `edit_result` at durable acceptance |
| `steer { steerId, content }` | Add input to the running turn | `steer_result` |
| `cancel_pending_steer { steerId }` | Withdraw a pending steer | None; the `pending_steers` list |
| `steer_now { steerId }` | Deliver a pending steer now ([Send now](#send-now)) | None; the transcript and the `pending_steers` list |
| `dequeue_message { messageId }` | Remove a queued message | `queue` |
| `send_queued_message { messageId }` | Send a queued message now ([Send now](#send-now)) | `queue` |
| `clear_message_queue` | Remove every queued message | `queue` |
| `abort` | Stop one thing ([Stop](#stop)) | `abort_result`, in request order |
| `abort_subagents`, `abort_subagent { subagentId }` | Stop subagents ([Abort](subagents.md#abort)) | `subagent` frames |
| `retry`, `resume`, `compact` | Run the action | `rejected` when the session is busy |
| `shell_write { commandId, stdin }` | Write stdin to a running command | `shell_write_result` |
| `shell_abort { commandId }` | Stop a running command | None; the command's `shell_output` shows its end |
| `sync_transcript { from, edge }` | Ask for a fresh transcript, from the page's blocks as `open` names them | `transcript_reset`, the subagent replay, `shell_output` for each live command, to this connection alone |
| `close` | Dispose the tree | `closed`, to every attached connection |

Content in `send`, `steer` and `edit_and_send` is typed. It is text, a
`reference`, an `upload` of a file the page already uploaded
(`{ type: "upload", ref, fileName }`), a `remote_file` on a paired device
(`{ type: "remote_file", deviceId, path }`), or, in `edit_and_send`, what the
edited message already holds: its native media by blob reference
(`{ type: "media", media: { type: "image", ref, mediaType } }`, a `video`
alike, and a `document` with its `fileName`) and its attachment records by
path (`{ type: "attachment", path }`)
([Files the edit keeps](message-editing.md#files-the-edit-keeps)). A `send`
or `steer` that holds `media` or `attachment` is an invalid frame. No frame
carries file bytes. The backend
resolves uploads and remote files before the session sees the content
([Media by reference](../backend/backend.md#media-by-reference),
[Device files and remote references](../product/web-api.md#device-files-and-remote-references)),
and the session resolves an edit's kept files from the edited message
([Files the edit keeps](message-editing.md#files-the-edit-keeps)).

`shell_write` and `shell_abort` reach the command through the shells of the
node that runs it, the root or a live subagent, for the conversation's current
Host, with the handle checks of [Running shell tools](#running-shell-tools).

### Server frames

| Frame | Carries |
| --- | --- |
| `opened` | The connection is attached |
| `transcript_reset` | `start`, the blocks from that index to the end, whole, the transcript's `length`, the version `{ epoch, revision }`, and `failures` ([Where a reset starts](#where-a-reset-starts)) |
| `transcript_patch` | Patches, the new revision, and `failures` |
| `phase` | `idle`, `running` or `compacting` |
| `context_usage` | The estimate of the root's next request in `tokens`, the `window` in use and `compactFrom`, the estimate from which `compact` is taken; both null for a model without a window. Right after the open handshake when the session can tell it without reading a blob ([Context estimate](compaction.md#context-estimate)) |
| `queue` | The queued messages, each `{ id, content }` |
| `pending_steers` | The complete list of pending steers |
| `steer_result` | The steer id and an `outcome`: `{ status: "accepted" }` or `{ status: "rejected", reason }` |
| `edit_result` | The operation id and an `outcome`: `{ status: "accepted", turnId }` or `{ status: "rejected", reason }` |
| `abort_result` | What was stopped, and whether another `abort` would stop more |
| `shell_output` | A command's live view ([Live output](#live-output)): `subagentId` when the command is a subagent's, and its `status`: `running`, `exited` with the `exitCode`, or `aborted`, each with the `commandId`, the `toolUseId` of the `shell` call that started it, the `tail` and `chars` of the pages' view, and `runningMs` |
| `shell_write_result` | The command id |
| `pending_calls` | The calls the model is writing, each `{ toolUseId, toolName, description }`, `description` null until written, and `subagentId` when they are a subagent's ([Calls being written](#calls-being-written)) |
| `retry_scheduled` | The attempt, the delay in milliseconds, the code and the diagnostics of a failure being retried ([Retries](failures-and-recovery.md#retries)) |
| `error` | A message, a code and diagnostics: a failed turn, a refused frame, or a failed save |
| `rejected` | The refused command and the reason |
| `subagent`, `subagent_transcript_reset`, `subagent_transcript_patch` | Subagent lifecycle and transcripts ([Protocol](subagents.md#protocol)) |
| `closed` | The connection is detached: its tree was disposed, or it sent `close` while attached to none |
| `heartbeat` | Nothing: the connection sent no other frame for 30 seconds ([Order and delivery](#order-and-delivery)) |

`failures` is the backend's reading of the error blocks a frame carries,
attached when the frame is sent and never stored
([Failure facts](../backend/backend.md#failure-facts)). Blocks, queued
messages and pending steers hold their media by blob reference, so no frame
carries media bytes ([Media](#media)).
Every `shell_output` is a command's live view, the same for every page: it
reports a change of a live command of any node of the tree, or answers an
attach or a `sync_transcript` ([Live output](#live-output)). No tool sends
one, and no read moves the model's place. Besides their commands' output,
child sessions send only their transcript frames. After a transcript reset,
a live command's `shell_output` carries its current status, so a command that
ended while no page watched shows as ended.

### Where a reset starts

A reset sends the blocks the page needs from the live tree and none it
already holds. For example, a page holds blocks 2,940 to 2,999, which it read
from the database, and opens the stream with `from: 2999`. The live tree has
gone on: block 2,999, the answer being written, has grown, and blocks 3,000
and 3,001 are new. The reset starts at 2,999 and carries three blocks; the
page replaces its block 2,999 and adds the rest.

- `from` is the index of the first block at the end of what the page holds
  that may have changed since the page read it: its first call that was
  executing, or else its last block, which may be the one being written.
  `edge` is that block's id. A reset starts at `from` and replaces the
  blocks from there whole.
- A block at `from` that is not `edge`, or a `from` past the transcript's
  end, means the transcript was rewritten between the page's read and the
  stream: the reset then starts at the start of the latest page, and the page
  drops what it held and takes the reset as its latest page
  ([Rewrites](../product/web-application.md#rewrites)).
- Without `from`, as when the page has read nothing, the reset starts at the
  start of the latest page.
- A subagent's reset (`subagent_transcript_reset`) starts at the start of its
  latest page, as the page holds none of a subagent's blocks until it shows
  it ([Protocol](subagents.md#protocol)).

The page applies a patch to the blocks it holds. A patch to a block the page
does not hold, between two parts of the transcript it read, changes nothing
the page shows; the page reads that block as it is when it reads its page,
and an `add` or a `truncate` moves the indices of the blocks after it
whether the page holds them or not.

### Order and delivery

- Client frames are strict: an unknown field is refused. The client accepts
  server frames with fields it does not know.
- A frame that does not match its schema is answered with an `error` whose code
  is `invalid_frame`, and the connection stays open. A message that is not JSON,
  or a binary one, closes the connection with close code 1007 `not_json`.
- The backend handles one connection's frames one at a time, in arrival order.
  A frame waits for the conversation's admission and is refused while the
  conversation is archived
  ([Sidebar mutations and read state](../product/web-api.md#sidebar-mutations-and-read-state)).
  A frame whose handling waits, such as a send whose uploads are being written
  to the Host or an edit waiting for its durable acceptance, delays the frames
  behind it.
- Every frame for one attachment, whether a reply or an event, goes through
  one outbox in causal order. The outbox holds at most 4,096 frames. When it is
  full, the server closes that connection with close code 4001 `lagged`, and
  the tree's other attachments go on as before; the client reconnects and
  adopts the running tree. A backend that shuts down closes its conversation
  sockets with 1001 `backend_closing` before it disposes the trees.
- The backend sends a `heartbeat` on a connection it has sent nothing else
  on for 30 seconds, whether the connection is attached or not. The socket
  sends it, not the tree: it does not go through the outbox and changes
  nothing the client holds. It lets a
  page tell a quiet connection from a dead one. For example, a laptop sleeps
  and its network drops without a close: nothing tells the page, whose socket
  still looks open. A page that receives nothing on its connection for 75
  seconds, two and a half heartbeats, takes the connection as broken, closes
  it and opens the conversation again
  ([Liveness and reconnection](../product/web-application.md#liveness-and-reconnection)).
- The open handshake is one step: nothing can happen to the session between
  `opened` and `pending_calls`, so the snapshot frames agree with each other.
- Each `transcript_patch` carries the revision one past the previous frame's.
  The client applies a patch whose revision is one past its own, ignores one
  whose revision is not greater than its own, and sends `sync_transcript` when
  it sees a gap. Subagent transcripts follow the same rule.

Without an open session, most commands are answered with `rejected`
(`No session is open`), `steer` with a rejected `steer_result`, and
`cancel_pending_steer`, `steer_now`, `abort_subagents` and `abort_subagent`
with nothing. A second `open` on one connection is rejected.

`ConversationClient` validates every frame it receives with the generated schemas and
drops the connection when one does not match, applies patches with the one
patch applier to the parts of the transcript the page holds, and keeps them,
the phase, the queue and the pending steers.

### Connections and the live tree

For example, a conversation is open in two pages, A and B, each with its own
connection. A sends a message: both pages show it, the reply as it streams and
the tool calls. B presses Stop while the turn runs: B alone receives the
`abort_result`, and both receive the stopped marker and the phase `idle`.

```text
connection A --+                    +--> A's outbox: events, and A's replies
               +--> the live tree --+
connection B --+                    +--> B's outbox: events, and B's replies
```

- A tree has any number of attached connections. `open` attaches its
  connection without detaching another, and the connection receives its own
  handshake ([Order and delivery](#order-and-delivery)), then every event of
  the tree.
- An event goes to every attached connection: the transcript, phase, queue,
  pending-steer, retry and subagent frames, the `error` of a failed turn, save
  or subagent lifecycle, and every `shell_output` of a live command's change
  ([Live output](#live-output)). A reply goes only to the
  connection whose frame it answers: `rejected`, an `error` that refuses the
  frame, `steer_result`, `edit_result`, `abort_result`, `shell_write_result`,
  and the frames that answer `sync_transcript`.
- A connection that closes only detaches. The tree's turns keep running, and
  the next `open` adopts the same live tree. Two concurrent opens of one
  conversation share one tree. A page that took its connection as broken for
  its silence ([Order and delivery](#order-and-delivery)) opens a new one,
  which adopts the tree the same way.
- `open` names no model. The backend opens the tree with the model selection
  the conversation's record holds, and a live tree already follows that
  record, so an open changes no model
  ([the conversation's model settings](../product/web-api.md#sidebar-mutations-and-read-state)).
  An open of a conversation whose record has no model yet is answered with an
  `error` whose code is `model_not_selected`, and one whose provider entry the
  user may no longer use with `provider_not_found`. A runtime the tree needs
  for that selection, such as a restored tree's, is built before the
  connection attaches, and a failure leaves the connection unattached.
- `close` disposes the whole tree. Every attached connection receives what the
  disposal changed, then `closed`, and is detached; a connection that sends
  `close` while attached to nothing receives `closed` alone.
- A tree that has been detached and quiescent (no action running or waiting,
  no live subagent, no command of any node running) for
  10 minutes is disposed; an `open` or new activity within those 10 minutes
  keeps it live. For example, a dev server the agent started goes on serving
  after the user closes the page, and the tree's 10 minutes start once it
  ends. The next `open`
  restores it from the store exactly as it was saved.

Connections that act at once need no rule of their own. The backend hands
each connection's frames to the session one at a time, and the session takes
the frames of all connections in the order they reach it, so the rule that
decides a lone client's frame decides each:

- Two sends run in the order they arrive; the later one waits in the queue
  ([Messages and the queue](#messages-and-the-queue)).
- An edit and a send: a send that arrives while the edit is being prepared is
  refused, and an edit that arrives while a send's turn runs or waits is
  refused ([Admission](message-editing.md#admission)). An edit against a
  transcript that another connection has changed since is stale
  ([Commit and idempotency](message-editing.md#commit-and-idempotency)).
- Two `abort` frames stop one thing each, in the order they arrive
  ([Stop](#stop)): the second can stop a queued message, and its
  `abort_result` says what it stopped.
- No frame changes the model selection: the conversation's model settings
  change by a conversation patch, which the backend applies one at a time
  ([the conversation's model settings](../product/web-api.md#sidebar-mutations-and-read-state)).

## Tree store

The tree store is the persistence contract between the agent and a
conversation's database. One store holds one conversation's tree: every node
with its parent link and its checkpoint. The backend realizes it over the
conversation's database
([Conversation state and transactions](../backend/storage.md#conversation-state-and-transactions)).
Tests use an in-memory store that meets the same contract. A node's store also
gives its session the conversation owner's blob namespace, where the session
puts the media that enter its transcript and reads back those its requests
send ([Media](#media)): a put names bytes by their SHA-256, and a get answers
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
  batch of patches ([Patches and versions](#patches-and-versions)).
- A save writes its rows as they are. Their media are references whose blobs
  were stored when the media entered, and a block or a queued message has no
  place for bytes ([Media](#media)), so a save stores no blob.

### Restoring

- The store decodes and validates every row it reads, and corrupt data stops
  the restore ([Storage](../backend/storage.md)).
- The session adds its own checks: the waiting agent messages have unique
  ids that are not already in the transcript, each is addressed to this node,
  and edit operation ids are unique.
- The session then completes interrupted tool calls and hands back its queue
  as [Dispose and restore](#dispose-and-restore) describes.
- A restore reads no blob: the session reads the blobs of its replayed media
  before its first request ([Media](#media)).

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
| Tool calls | Input refusals, the repeat guard, the cut of a result, handle release and binary stdout verdicts match [Tools](#tools) |
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
