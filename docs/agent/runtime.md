# Agent runtime

The agent runtime runs the agents of every conversation. Each agent is a
session: it keeps a transcript, asks a provider for the next response, runs the
tools the model requests, and saves its checkpoint in the conversation's
database. The browser follows and controls a conversation over one WebSocket
that carries agent frames.

For example, a user sends "run the tests and fix what fails":

```text
browser (AgentClient)        backend: the user's shard                   Host
---------------------        -------------------------                   ----
send ----------------------> conversation socket
                               -> tree -> root session
                                    user block
                                    provider request
                                    text + shell_exec call
                                    save, run the tool ----------------> runner job
                                    tool result <----------------------
                                    provider request
                                    text + response: the turn ends
transcript_patch <---------- one outbox per connection
                               tree store -> conversation database
```

The root session appends the message as a `user` block and asks the provider
for a response. The model answers with some text and a `shell_exec` call. The
session saves the call as executing, runs the command on the conversation's
Host, records the result, and asks the provider again. When a response
requests no tool, the turn ends. Each change reaches the browser as a
transcript patch and the conversation's database as changed rows.

This document owns sessions and turns, their input, yield wakeups, the
standard tools, the transcript and its views, the rendering boundary, the frame
protocol, and the tree store contract. Related rules have their own homes:

- The session tree, `demi agent` and agent messages:
  [Subagents](subagents.md).
- Compaction, token estimates and truncation:
  [Compaction](compaction.md).
- Provider failures, retries and recovering an unfinished turn:
  [Failures and recovery](failures-and-recovery.md).
- [Message editing](message-editing.md),
  [Conversation Fork](conversation-fork.md) and
  [Command state history](command-state-history.md).
- The thread a session runs on: [the user's shard](../architecture/concurrency.md#the-user-shard).
- The crates that implement it: [Crates](../architecture/crates-and-packages.md#crates).

## Sessions and turns

A session belongs to one agent node: the root of a conversation or one of its
subagents. Every node has the same kind of session, built by the same assembly
([Runtime](subagents.md#runtime)). A session holds its transcript and command
state, its model selection with its own provider runtime, the actions waiting
to run, the pending steers, the scheduled yield wakeups, and one status.
Sessions run on the user's shard thread, where no other work runs while a
session changes its state.

The harness supplies a node's system prompt, the text added to each of its user
turns (a subagent's identity, for example), the context text described in
[Transcript](#transcript), and its commands. Demi has one harness, the coding
agent.

### Actions

Work reaches a session as actions. One action runs at a time, and the others
wait in arrival order.

| Action | Comes from | What it does |
| --- | --- | --- |
| Send | A `send` frame, or the next queued message | Appends a `user` block and runs a turn |
| Continuation | A yield wakeup or an agent message while nothing runs | Appends a `wakeup` block, or the waiting agent messages, and runs a turn; never shown in the queue |
| Retry | A `retry` frame | Rewinds the last input turn and runs it again ([Recovery](failures-and-recovery.md#recovery-is-one-mechanism)) |
| Resume | A `resume` frame | Unwinds the unfinished turn to its resume point and continues it ([Recovery](failures-and-recovery.md#recovery-is-one-mechanism)) |
| Compact | A `compact` frame | Runs one compaction pass ([Compaction](compaction.md#compaction)) |
| Edit and send | An `edit_and_send` frame | Replaces a user message and everything after it, then runs a turn ([Message editing](message-editing.md)) |

Admission is decided when an action arrives:

- A disposed session refuses every action.
- While an edit is being prepared, the session refuses sends, the other
  actions, steers, agent messages and new yield wakeups. A model switch waits
  for the edit instead ([Model switch](#model-switch)).
- `retry`, `resume` and `compact` are refused unless the session is idle, with
  the reason `Session is busy (<phase>)`.
- A send is never refused because the session is busy: it waits in the queue
  ([Input](#input)).

A running action holds a lease on its tree's admission. A target switch or an
archive reserves the idle tree through the same admission
([Switch the main target](../execution/sessions-and-targets.md#switch-the-main-target)),
and a running turn is conversation activity
([Activity](../execution/resource-lifecycle.md#activity)).

### A turn

A send, a continuation, a retry, a resume and an accepted edit each run a turn:

```text
action starts
  prepare the input: a new block, or the rewound or unwound history
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
  discard human steers still pending, arm yield wakeups, save,
  start the next waiting action
```

A turn ends when a response requests no tool, or when a tool asks to end the
turn after its result, as `yield` does. Input that arrived during the round
overrides both: the turn asks the provider once more. Transient provider
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
3. Otherwise, the oldest scheduled yield wakeup is cancelled.
4. Otherwise, nothing is stopped.

The `abort_result` frame says what was stopped and whether another `abort`
would have stopped something more at the moment the stop was recorded.

A stopped action records the stop itself. It writes the human steers still
pending and the yield wakeups that fired, completes each running tool call as
an error `Tool call aborted: <tool>`, and appends an `abort` block, the
stopped marker. Agent messages keep waiting: the stop is the user's, and the
user's next action reads them
([Delivery and scheduling](subagents.md#delivery-and-scheduling)).
If the action was saving a history rewrite, it records the stop after the
rewrite is published, so a rewrite never loses a stop. `abort_result` is sent
only once the record is in the transcript. A provider run that is cancelled
ends without an event; the session, not the provider, records the stop.

### Dispose and restore

A session is disposed when its tree closes: a `close` frame, the eviction of an
idle detached tree ([Frame protocol](#frame-protocol)), or backend shutdown.
Dispose does the following:

1. Refuses new actions and invalidates the node's outstanding command-storage
   handles.
2. Stops the running action as a shutdown. The human steers still pending
   are written, running tool calls complete as
   `Tool call aborted: <tool>`, and an `error` block with the code
   `interrupted` and the message "The agent session was shut down while this
   turn was running." says why the turn is unfinished. A message whose turn
   has not written its `user` block yet is not interrupted: it goes back to
   the front of the queue.
3. Keeps the queued messages, the agent messages waiting for a boundary and
   the yield wakeups, scheduled or fired, in the checkpoint.
4. Waits for a save in progress, then writes the final checkpoint. Its phase
   is `running` when a turn was interrupted.
5. Closes the provider runtimes, including the one a pending model switch had
   built.

Restoring reads the checkpoint ([Tree store](#tree-store)). A tool call still
marked executing completes as an error
`Tool call interrupted: <tool> (the process died before a result was recorded)`:
its outcome is unknown, and it never runs again
([Recovery and persistence](../execution/sessions-and-targets.md#recovery-and-persistence)).
The restored session is idle, with its saved wakeups armed
([Yield wakeups](#yield-wakeups)). It hands back its queued messages and
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
  next action, or inside a running turn at its next continuation boundary. A
  request that is streaming finishes with the model it started with. A
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
- after each tool call, unless the turn is about to compact.

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
- `dequeue_message` removes one message. `send_queued_message` moves one to the
  front, so it runs next. `steer_queued_message` turns one into a steer of the
  running turn, and puts it back in its place when the steer is refused.
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
  marker. A failed action writes all pending input before it ends. A human
  steer still pending when its action ends normally is discarded; agent
  messages are kept for the next continuation.

### Pending steers

The session keeps the list of accepted human steers that are not yet in the
transcript. Each entry holds:

| Field | Meaning |
| --- | --- |
| `id` | The steer id from the `steer` frame. The `steer` block uses the same id. |
| `turnId` | The running turn that receives the steer. |
| `model` | The model selection when the steer was accepted. |
| `content` | The steer's content, attachments included. |

The list never contains yield wakeups or agent messages.

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

`AgentClient` keeps the current list and exposes it with its `pendingSteers()`
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
`AgentClient.open()` resolves on the `opened` frame; the snapshot frames follow
it, so a caller that needs the initial list subscribes to `pending_steers`,
which also reports an empty list.

## Yield wakeups

For example, the model starts a build that takes minutes and calls `yield` with
`durationMs: 120000`. The turn ends after that round of tools. Two minutes after
the turn ended, the wakeup fires. Nothing is running, so the session starts a
continuation whose input is a `wakeup` block, and the model checks the build
with `shell_status`.

- `yield` returns an effect for the session to apply: schedule one wakeup and
  end the turn after this round of tools. The tool result says
  `yield scheduled` with the wakeup id and the duration, and its view is
  `yield_wakeup`.
- The wait starts when the action ends, not when `yield` is called.
- When a wakeup fires during a turn that accepts steers, it joins that turn at
  the next continuation boundary as a `wakeup` block with the placement
  `steer`. Otherwise it starts a continuation whose input is a `wakeup` block
  with the placement `new_turn`. Either way, the model receives the text
  "Scheduled yield wakeup fired. Continue the previous work and inspect any
  running command with shell_status when needed." A wakeup never appears in
  the queue or among the pending steers.
- A wakeup belongs to its session, not to the turn that scheduled it: a turn
  the user started meanwhile receives it like its own.
- When no action runs or waits, Stop cancels the oldest scheduled wakeup
  ([Stop](#stop)).
- A wakeup is saved in the checkpoint with its id, its duration and, once the
  action that scheduled it has ended, the wall-clock time it is due, so it
  survives dispose and a backend restart; a fired wakeup stays saved until
  the transcript holds it. Restoring the session arms it again: a wakeup
  whose time passed while the session was not live is due at once, and one
  whose action the process died in starts its wait at the restore. A backend restart opens no conversation: a saved
  wakeup waits until its tree is next restored. A restored root whose last
  turn was interrupted holds its due wakeups as it holds its pending agent
  input ([Persistence](subagents.md#persistence)).
- A scheduled wakeup is not conversation activity: it keeps no Cloud awake
  ([Activity](../execution/resource-lifecycle.md#activity)). A subagent with a
  scheduled wakeup stays live ([Result](subagents.md#result)), and message
  editing is refused while a wakeup is scheduled
  ([Message editing](message-editing.md)).

## Tools

The model has five tools, and only these:

| Tool | What it does |
| --- | --- |
| `shell_exec` | Starts a script in a shell on the conversation's Host and watches it for up to `timeoutMs`. The window is for watching, not a deadline: a command still running when it ends keeps running, and the result carries its handle (`commandId`). Completed short output returns directly. |
| `shell_status` | Reads a running command's status and the output since the model last looked. It neither waits nor writes. |
| `shell_write` | Writes non-empty stdin to a running command and returns its status with the new output. |
| `shell_abort` | Stops a running command. Its result is never an error. |
| `yield` | Ends the turn and schedules one wakeup after `durationMs` ([Yield wakeups](#yield-wakeups)). |

Everything else the agent does runs as commands in the shell, such as
`demi file`, `demi todo`, `demi agent`, `demi browser` and `demi host`
([Commands](../execution/commands.md)). A tool call whose name is not one of
the five completes as an error `Tool not found: <name>`.

### Tool input

- Each tool declares its input once. The JSON Schema the model receives and the
  check each call runs come from that one declaration. The schema's dialect
  declaration is dropped, because a provider embeds the schema in its own
  request.
- Unknown fields and wrong types are refused. A refused call completes as an
  error whose text starts with `<tool> input is invalid:` and names each
  offending field, so the model can correct the call.
- `timeoutMs` and `durationMs` are whole milliseconds from 1 to 600,000,
  declared to the model as `integer`. A fraction is refused, not rounded.
- Every tool accepts an optional `description`, the call's title for the user
  ([Rendering boundary](#rendering-boundary)).

### Running shell tools

- The shell tools reach the conversation's current Host through the
  conversation's host access
  ([Host operations](../execution/sessions-and-targets.md#host-operations)).
  Their jobs run in the runner's shell
  ([Shell jobs](../execution/runner.md#shell-jobs)).
- Each node keeps one shell environment per Host it has used. Concurrent calls
  for one Host create one environment.
- A handle belongs to its environment. A `shellId` or `commandId` of another
  Host's environment is refused with `Shell handle "<id>" belongs to a
  different Host`, and a handle that two environments claim is refused as not
  unique.
- The repeat guard counts identical scripts. In one environment, a `shell_exec`
  of the same script within 60 seconds of the previous one is allowed six times
  in a row. The seventh and later identical calls do not run: the result is an
  error that asks the model to inspect the previous output, and its view is
  `repeated_shell_exec` with the script and the count. Another script resets
  the count.

### Results and previews

- The model keeps its own place in each command's output: a result shows the
  output since the model's last look at the command. What the pages are sent
  never moves it ([Live output](#live-output)). For example, a page shows a
  running command's new output; the model's next `shell_status` still shows
  all of it.
- A result gives the command's status and exit code, its handle and timings
  when the handle matters, a preview of the output, and a hint for the next
  step.
- A result's `idleMs` counts from the last time the command's output grew,
  also beyond the first 32 KiB of a stream, which the model's view does not
  hold: the runner reports such growth within 2 seconds even while no page
  follows the command
  ([Pipes and output](../execution/runner.md#pipes-and-output)). For example,
  a build that prints its 100th KiB of log lines a second ago shows an
  `idleMs` below 3,000, so the model does not take it for a hung one.
- The preview is the start of the merged output, up to four characters per
  budget token. The budget is 10,000 tokens when the request's model has a
  context window below 800,000 tokens, and 100,000 tokens at or above it. It
  applies to every node of the tree, with each request's current model.
  Characters are Unicode scalar values
  ([Token estimates](compaction.md#token-estimates)).
- The result carries the command's handle, timings and output file paths when
  the command still runs or its output did not fit: the preview was cut, the
  output exceeded the budget, or the runner's own view of a stream was
  truncated. Otherwise the call has shown everything, and the handle is
  released.
- When a command exits with binary stdout, the result attaches it as an image
  or a video only when the stream is complete, its bytes are a media type of
  the model-media table, and the model accepts that type. An image is attached
  as it is fitted ([Images in the transcript](#images-in-the-transcript)). A
  video is attached up to 16 MiB, and only when its base64 takes at most half
  of the model's request body limit
  ([Request limits](../providers/models.md#request-limits)), so that a request
  still has room for the history around it. Otherwise the result says why
  nothing was attached and where the raw bytes remain readable.

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
  |       the first 32 KiB of each stream, always
  |       beyond them, while followed: the newest bytes,
  |       at most 16 KiB per stream every 250 ms;
  |       while not: the stream's length, at most every 2 s
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
  end adds: the end of each stream that the backend had not received
  ([Pipes and output](../execution/runner.md#pipes-and-output)), why the
  command could not run, or a binary stdout's description.
- **Which commands.** Every command of the tree, the root's and each live
  subagent's, several at once, on a Cloud and on a paired device alike: both
  are Hosts behind a runner.
- **While a page is attached.** While at least one connection is attached to
  the tree ([Connections and the live tree](#connections-and-the-live-tree)),
  the backend follows every running command of the tree: the runner then sends
  each stream's output beyond its first 32 KiB as well. A command's start and
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
  following and sends nothing; the runner sends each stream's first 32 KiB
  again and, beyond them, only the stream's length, which the model's idle
  time counts from ([Results and previews](#results-and-previews)). The next
  connection to attach starts from its handshake.
- **The end.** A command's last frame shows its end: its exit, a stop (a
  `shell_abort` from a page or from the model, a Stop of the action that
  started it), or the end of its node's shells: the close of its subagent, the
  tree's interruption for a Host transition, or the tree's disposal, which
  ends the tree's commands before its connections receive `closed`. No frame
  of the command follows it.

What keeps the output coming, and where each part is released:

| Part | Lives in | Released |
| --- | --- | --- |
| The runner's messages beyond a stream's first 32 KiB, with their timer | The job's task on the runner | With the job; the timer runs only while a message waits |
| The backend's following of a job: it watches whether a page is attached and tells the runner | The job's task in the node's shell environment | With the job |
| The commands whose new output is not sent yet | The tree | Emptied when sent, when a command ends, and when the last connection detaches |
| The task that sends them | The tree; it has a timer only while commands wait | With the tree |
| The reports of new output and ends | The tree's feed, which each shell environment of each node holds without keeping the tree alive | With the environment |

### Dispatch and failures

- Before the first tool of a round runs, the session saves its checkpoint, so
  every call it is about to run is stored as executing. A process that dies
  during a tool leaves the call executing, and restore completes it as
  interrupted without running it again ([Dispose and restore](#dispose-and-restore)).
- Tools run one at a time, in the order the model requested them. Waiting
  input is written after each call ([Input](#input)).
- A tool that fails completes its call as an error `Tool failed: <message>`.
- An action that fails before its calls ran, such as when the save before
  dispatch fails, completes them as `Tool call aborted: <tool>`, as a stop
  does, so the next request replays no call without a result.
- A tool never reaches into its session. It returns its result and, for
  `yield`, an effect that the session applies.

## Transcript

Words used for session data:

| Word | Meaning |
| --- | --- |
| transcript | A session's history: its ordered blocks |
| status | A point-in-time answer an operation returns, such as a command's status; never stored as history |
| checkpoint | The durable, restorable state of one session ([Tree store](#tree-store)) |
| retained output | The complete output of one command, kept as files on the device that ran it ([Pipes and output](../execution/runner.md#pipes-and-output)) |
| view | Bounded data a block carries for the user; never replayed to the model |
| blob | Content-addressed bytes, such as media, in the conversation owner's blob namespace |

### Block types

| Block | Written by | The model receives | The user sees it |
| --- | --- | --- | --- |
| `user` | A send or an edit: the submitted content and the harness's text for the turn (`preamble`) | A user message: the preamble, then the content | Yes; the only editable block ([Message editing](message-editing.md)) |
| `context` | The session before a provider request, when the conversation's execution context changed since the node last saw it: a target switch, attached hosts, a Cloud reset ([Switch the main target](../execution/sessions-and-targets.md#switch-the-main-target)) | A user message with its text | No |
| `wakeup` | A fired yield wakeup, with the placement `new_turn` or `steer` | The fixed wakeup text, as a user message or as a steer | No |
| `steer` | A human steer, at a continuation boundary | A steer in the current turn | Yes |
| `agent_message` | Another agent of the tree ([Communication](subagents.md#communication)) | A steer holding the message's source envelope | As a receipt row |
| `resume` | A turn continuing after a cut: `resume`, compaction inside a turn, or a model switch that landed inside a turn and compacted | A user message: "Continue from where you left off." | No |
| `abort` | Stop | Nothing | Yes, until the turn is continued and `isResumed` is set |
| `thinking` | The provider: reasoning text and its signature | The text; a signed block whole | Yes |
| `redacted_thinking` | The provider: opaque reasoning data | The data, whole | No |
| `text` | The provider: assistant text, marked `forkable` once complete ([Conversation Fork](conversation-fork.md)) | The text | Yes |
| `tool_call` | The provider's call, completed by the session with the result | The call and, once completed, its result | Yes |
| `response` | The provider: the usage of one completed request | Nothing; its usage anchors the context estimate | No |
| `error` | A failed request or an interrupted turn ([The failure record](failures-and-recovery.md#the-failure-record)) | Nothing | Yes |
| `compaction_boundary` | Compaction: the summary, inserted where the kept history begins | A user message: "Previous conversation summary:" and the summary | Yes |
| `compaction_marker` | Compaction: the estimated size of what was summarized, appended at the end | Nothing | No |

A `user`, `context` or `wakeup` block with the placement `new_turn` opens an
input turn: recovery treats it as the start of its turn
([Recovery](failures-and-recovery.md#recovery-is-one-mechanism)). For retry,
the first `agent_message` of a continuation opens its turn as well. The other
blocks belong to the turn they appear in.

A `tool_call` block holds the provider's `toolUseId` and `toolName`, the call's
`input` as the JSON text the provider supplied, its `status` (`executing`,
`completed` or `error`), its `output`, and its `view`.

### Replay

The model receives the blocks from the last `compaction_boundary` onward, the
replayed blocks, each as the table says. A long text is cut in the middle by
the replay bound ([Text bounds](compaction.md#text-bounds)). Signed thinking
and redacted data are replayed whole, because the vendor verifies them as they
were sent. Each medium is replayed with the bytes the session holds for it
([Media](#media)). A tool call's input is replayed as the JSON value the
provider supplied, or as text when it is not valid JSON.

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

A message's images, videos and documents and a tool result's images and
videos are stored once, as blobs in the conversation owner's blob namespace,
and blocks hold them by reference: `{ type: "ref", ref, mediaType }`, with a
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
  A tool's medium whose put fails becomes the text
  `[<kind> not stored: <reason>]` in its result, so the turn goes on and no
  block names a blob that was not stored.
- **References.** The transcript, the queued messages, the pending steers,
  every checkpoint row and every frame hold media only by reference. Nothing
  converts media when a block is saved or sent, a store refuses a checkpoint
  that holds media bytes ([Saving](#saving)), and an edit or a Fork copies
  references.
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
  replayed blocks with each medium's held bytes in place of its reference
  ([Replay](#replay), [Token estimates](compaction.md#token-estimates)); a
  provider request is the only place a medium carries its bytes. Before the
  session builds that view, it reads the blob of each replayed medium it
  holds nothing for, which after a restore is every one, a few at a time
  concurrently, since on S3 each read is a round trip. Opening a
  conversation therefore reads no blob, a request never reads a blob the
  session holds, and no blob from before the last `compaction_boundary` is
  read. A blob that is missing becomes the text `[missing <kind> blob <ref>]`
  in the view, so the turn goes on; the transcript keeps the reference.
- **Stable requests.** Within a live tree, a medium reaches the model in one
  form for as long as it is replayed: its held bytes, or the missing text for
  a blob found missing. Bytes the session let go of and reads again are the
  same bytes, since a blob's name is their hash and a blob that a block, a
  queued message or a pending steer references is never deleted
  ([Collecting blobs](../backend/storage.md#collecting-blobs)). Media therefore never
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
([Results and previews](#results-and-previews)), or when a message's upload
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

The original stays where it came from: a tool's stdout in the command's
retained output on the Host, whose path the result names, and an upload in its
attachment file on the Host and in its upload blob, whose attachment record
keeps the original's size and hash. Fitting decodes and encodes on the
blocking pool ([Blocking work](../architecture/concurrency.md#blocking-work)).
Videos and documents are not fitted; they count toward the request's size
([Request size](compaction.md#request-size)).

### Views

A `tool_call` block's `view` carries bounded data for the user. It is never
replayed to the model, it never embeds unbounded payloads such as full output,
file bodies or raw bytes, and its type is fixed per tool by `kind`:

| `kind` | Fields |
| --- | --- |
| `shell` | `status` (`running`, `exited` or `aborted`); `shellId`; `commandId`; `exitCode`, once exited; `runningMs`; `idleMs`; `chunks`, the last 32,768 characters of the merged stdout and stderr, each chunk tagged with its stream; `viewTruncated`, true when that window or the output itself was cut; `files` and `filesTruncated`, once the command has exited and changed files |
| `repeated_shell_exec` | `script`, `count` |
| `yield_wakeup` | `wakeupId`, `durationMs` |

The shell view's characters are Unicode scalar values, counted from the end so
the newest output stays. `files` lists one entry per changed path with its line
counts and, for each edit segment, whether its contents were kept; the contents
stay in the change store ([Edit tracking](../execution/edit-tracking.md#the-change-store)).
The model's result and the view come from the same command status; the view
shows nothing the model could not read from the command, except `files`, which
exists only for the user.

### Retired tool media

A tool result's image or video stays in its block for 30 days. After that it
is retired, once no request can send it while a vendor may still keep that
request in its cache: it gives way, in its place in the result, to one line of
text that says what it was and when it was removed. For example, a screenshot
a command printed on 1 September, in history that compaction summarized on 3
September, gives way at the first daily pass after 1 October to:

```text
[image:image/png, removed on 2026-10-01: a tool result's images and videos are kept for 30 days]
```

A medium is retired when its `tool_call` block is more than 30 days old and
one of these holds:

| Where it lies | Why no cache can still hold a request that sent it |
| --- | --- |
| Before its node's last `compaction_boundary`, and that boundary is more than 24 hours old | Replay starts at the boundary, so no request has sent the medium since the summary request, more than a day ago, and no vendor keeps a cache entry longer than 24 hours. An edit or a retry that removes the boundary later sends the text into a cache that has ended |
| Anywhere in a conversation that has been idle for 30 days | No request of the conversation has been sent for 30 days ([Retiring tool media](../backend/storage.md#retiring-tool-media) says how the backend knows) |

- The text is `[<kind>:<media type>, removed on <date>: a tool result's images
  and videos are kept for 30 days]`. The kind is `image` or `video`, and the
  date is the UTC day of the retirement, `YYYY-MM-DD`. The text replaces the
  image or video part of the block's `output`; the result's other parts, its
  `view`, and the block's id, time and status stay.
- The model receives the text as any text part of a result. The backend
  retires only conversations without a live tree
  ([Retiring tool media](../backend/storage.md#retiring-tool-media)), so
  within a live tree a medium reaches the model in one form for as long as it
  is replayed. A request after an idle month carries the text where the
  previous one carried the image.
- A message's images, videos and documents are never retired: they come from
  the user's uploads.
- The page shows the call as before. The renderers of the five tools draw its
  stored `view`, which never held the image
  ([Rendering boundary](#rendering-boundary)), and the generic tool card shows
  the text as it shows any text part.
- The agent owns the rule: which media are retired, when, and the text. The
  backend applies it to stored conversations
  ([Retention](../backend/storage.md#retention)).

### Patches and versions

Every change a session makes to its transcript is recorded as patches, each
naming the index of the block it touches:

| `op` | Fields | Meaning |
| --- | --- | --- |
| `add` | `index`, `value` | Insert a block at the index |
| `replace_block` | `index`, `value` | Replace the block at the index |
| `append_text` | `index`, `delta` | Append text to the block at the index |
| `replace` | `value` | Replace every block |

Consecutive appends to one block merge into one `append_text`. A rewrite of
history, such as a retry, a resume's unwind or an accepted edit, is one
`replace`. Each batch of patches advances the transcript's revision by one.

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
3. Each of the five tools has its own rendering; none falls through to the
   generic tool card.
4. The generic tool card is only for a tool name the runtime does not have. A
   model can request one; its call then ends with `Tool not found`.
5. The renderers live in `web-ui`. `web` and `web-gallery` feed them the same
   blocks and frames, and neither defines a second data model.

A renderer uses the `tool_call` fields as [Transcript](#transcript) defines
them: `toolName` selects the rendering, `input` is JSON text the renderer
parses, `status` and `output` give the result, and `view` enriches the display
with the command's own output and status.

Live frames add to the transcript; they do not replace it:

- `transcript_reset` and `transcript_patch` are the primary input.
- `shell_output` adds a command's live output and status
  ([Live output](#live-output)). While the `shell_exec` call that started the
  command runs, the page shows them under that call, which the frame names
  (`toolUseId`, in the subagent's transcript when `subagentId` is set). Once
  the call has returned, a command that still runs is one of the
  conversation's running commands: the dock's Running chip counts it, its
  panel shows it under its script, and its output keeps coming there, while
  the call keeps the view its result stored. A page adds to what it shows
  only the characters beyond those it has shown (`chars`), so a terminal
  keeps its scrollback; after a gap it shows the `tail` anew. A command's tab
  shows its status as its last frame, or after a reload its stored view,
  gives it: running, exited, or stopped (`aborted`). A command the user or
  the agent stopped did not finish, so it is never marked done.
- `shell_write_result` and `abort_result` acknowledge the user's controls.
  They do not mean the command or the turn has finished, and they do not
  replace the `tool_call` rendering.

A patch replaces a block at its index, and renderers key blocks by their id,
so an update never shows a second record. Renderers tell shell execution,
status reads, stdin delivery, command cancellation and waiting apart by tool
name. Component structure, expansion, icons, typography, motion and the
presentation of changed files are shown in the gallery, not here.

### Tool descriptions

Every tool's input accepts an optional `description`: a short title, shown to
the user, for the concrete state or result the step makes visible, confirms or
advances. It names what the user will see, not how the tool works.

1. A non-empty `description` is the preferred title of the tool block.
2. Without it, the renderer uses the tool's own fallback title.
3. `description` affects display only. It changes neither the shell's
   behavior, the tool result, nor what the model receives on replay.
4. A `description` does not describe waiting, pausing or tool mechanics, is
   not a generic action or a bare noun, and does not hold scripts, output,
   protocol state, step numbers, tool names, ids, internal labels or reasons.

## Frame protocol

The browser opens `WS /api/conversations/:id/stream` for a conversation
([Web API](../product/web-api.md)), with the session cookie, from a page of
the product
([Authentication and ownership](../backend/backend.md#authentication-and-ownership)).
The connection belongs to that one
conversation: the backend supplies the session id and the working directory
from the conversation's target, and the client never sends them. The types of
every frame are Rust types, and the browser validates frames with the schemas
generated from them ([Generated TypeScript](../architecture/contracts.md#generated-typescript)).
`AgentClient`, in `@demicodes/agent-client`, is the browser's client of this
protocol.

For example, a page opens a conversation whose root is running:

```text
client                               server
open ------------------------------> attach to the live tree
                <------------------- opened
                <------------------- transcript_reset { blocks, version: { epoch, revision: r } }
                <------------------- phase, queue, pending_steers
                <------------------- subagent started + subagent_transcript_reset,
                                     for each live subagent, depth first
                <------------------- shell_output, for each live command
                <------------------- transcript_patch { revision: r + 1 }
```

### Client frames

| Frame | Meaning | Answer |
| --- | --- | --- |
| `open` | Attach this connection to the conversation's tree, restoring the tree when it is not live, with the model selection the conversation's record holds | `opened`, then the snapshot frames |
| `send { messageId, content }` | Submit a message ([Input](#input)) | Transcript, phase and queue frames |
| `edit_and_send { request }` | Replace a user message and its suffix ([Message editing](message-editing.md)) | `edit_result` at durable acceptance |
| `steer { steerId, content }` | Add input to the running turn | `steer_result` |
| `cancel_pending_steer { steerId }` | Withdraw a pending steer | None; the `pending_steers` list |
| `dequeue_message`, `send_queued_message { messageId }` | Remove a queued message; run one next | `queue` |
| `steer_queued_message { messageId, steerId }` | Turn a queued message into a steer | `steer_result` |
| `clear_message_queue` | Remove every queued message | `queue` |
| `abort` | Stop one thing ([Stop](#stop)) | `abort_result`, in request order |
| `abort_subagents`, `abort_subagent { subagentId }` | Stop subagents ([Abort](subagents.md#abort)) | `subagent` frames |
| `retry`, `resume`, `compact` | Run the action | `rejected` when the session is busy |
| `shell_write { commandId, stdin }` | Write stdin to a running command | `shell_write_result` |
| `shell_abort { commandId }` | Stop a running command | None; the command's `shell_output` shows its end |
| `sync_transcript` | Ask for a fresh transcript | `transcript_reset`, the subagent replay, `shell_output` for each live command, to this connection alone |
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
| `transcript_reset` | Every block, the version `{ epoch, revision }`, and `failures` |
| `transcript_patch` | Patches, the new revision, and `failures` |
| `phase` | `idle`, `running` or `compacting` |
| `queue` | The queued messages, each `{ id, content }` |
| `pending_steers` | The complete list of pending steers |
| `steer_result` | The steer id and an `outcome`: `{ status: "accepted" }` or `{ status: "rejected", reason }` |
| `edit_result` | The operation id and an `outcome`: `{ status: "accepted", turnId }` or `{ status: "rejected", reason }` |
| `abort_result` | What was stopped, and whether another `abort` would stop more |
| `shell_output` | A command's live view ([Live output](#live-output)): `subagentId` when the command is a subagent's, and its `status`: `running`, `exited` with the `exitCode`, or `aborted`, each with the `shellId`, the `commandId`, the `toolUseId` of the `shell_exec` call that started it, the `tail` and `chars` of the pages' view, and `runningMs` |
| `shell_write_result` | The command id |
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
  `opened` and `pending_steers`, so the snapshot frames agree with each other.
- Each `transcript_patch` carries the revision one past the previous frame's.
  The client applies a patch whose revision is one past its own, ignores one
  whose revision is not greater than its own, and sends `sync_transcript` when
  it sees a gap. Subagent transcripts follow the same rule.

Without an open session, most commands are answered with `rejected`
(`No session is open`), `steer` and `steer_queued_message` with a rejected
`steer_result`, and `cancel_pending_steer`, `abort_subagents` and
`abort_subagent` with nothing. A second `open` on one connection is rejected.

`AgentClient` validates every frame it receives with the generated schemas and
drops the connection when one does not match, applies patches with the one
patch applier, and keeps the transcript, the phase, the queue and the pending
steers.

### Connections and the live tree

For example, a conversation is open in two browser tabs, A and B, each with
its own connection. A sends a message: both tabs show it, the reply as it
streams and the tool calls. B presses Stop while the turn runs: B alone
receives the `abort_result`, and both receive the stopped marker and the
phase `idle`.

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
  no live subagent, no scheduled wakeup) for 10 minutes is disposed; an `open`
  or new activity within those 10 minutes keeps it live. The next `open`
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

A node's checkpoint has three parts:

| Part | Holds |
| --- | --- |
| Transcript rows | One row per block, by index |
| State row | The phase; the queued messages, each `{ id, content }`; the agent messages waiting for a boundary; the yield wakeups not yet in the transcript, each with its id, its duration and its due time once its action ended; the working directory; the model selection; the harness name; the accepted edit receipts |
| Command state | Its versions and boundary references ([Command state history](command-state-history.md)) |

Human pending steers are not part of it. Creating,
closing, reopening and deleting nodes, and delivering subagent completions, are
atomic commits of the same store ([Persistence](subagents.md#persistence)).

### Saving

- A save carries only what changed: the changed block rows in ascending index,
  the block count (rows at or beyond it are deleted), the state row, and the
  new command-state versions and boundaries. The store commits a save in one
  transaction.
- A session has one save in progress at a time. Saves, command-state commits,
  the boundary captured when assistant text completes, history rewrites and
  edit commits run in the order they were requested. A running turn never
  holds this order while it waits for a provider, a tool or the harness, so a
  tool's command-storage write can commit meanwhile.
- A save that has started always finishes: Stop does not cancel it, and it
  took effect exactly when the store reports success. Right before its
  transaction, the store checks the save's guard: a command-storage write from
  an invocation that a history rewrite or dispose has invalidated fails
  instead of committing ([Command state history](command-state-history.md)).
- A change schedules a save one second after the first unsaved change. These
  points save at once: before tools are dispatched, after a `context` block,
  when an action ends, when an agent message is admitted or written, on a
  history rewrite or an edit commit, and on dispose. The save at the end of an
  action records the phase idle, so a checkpoint that says an action was
  running is one the process died in; clients see the phase go idle only once
  that save has committed ([A turn](#a-turn)).
- A save is due when the transcript, command state, edit receipts, the queue,
  the waiting agent messages or the model selection changed. The phase alone
  never makes a save due.
- A scheduled save that fails is reported as an `error` frame, and its rows are
  saved with the next change. When the save at the end of an action fails, the
  action fails.
- A history rewrite is saved before it is published: the store commits the
  retained rows with the command state of the cut, then the session adopts them
  and publishes one `replace` patch.
- A save writes its rows as they are. Their media are references whose blobs
  were stored when the media entered ([Media](#media)), so a save stores no
  blob. A store refuses a save, or a node's first checkpoint, in which a
  block or a queued message holds media bytes instead of a reference: it
  answers an error that names the block or the message, and writes nothing.

### Restoring

- The store decodes and validates every row it reads, and corrupt data stops
  the restore ([Storage](../backend/storage.md)).
- The session adds its own checks: the checkpoint's harness is the node's, the
  waiting agent messages have unique ids that are not already in the
  transcript, each is addressed to this node, and edit operation ids are
  unique.
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
| A command prints while its `shell_exec` call runs | The attached connections receive the output before the call's result |
| A command prints after its call returned | Every attached connection receives the new output, with no frame from any client |
| A connection attaches while a command runs | Its handshake carries the command's view, whatever another connection or the model read; it then receives each change as the others do |
| A command prints faster than a page reads | A page receives at most one frame of it every 250 ms, and its outbox does not fill; the view shows the output beyond the first 32 KiB of each stream |
| A command ends | No frame of it follows the frame of its end |
| The last connection detaches | The runner sends no more output beyond the first 32 KiB of a stream, and the tree sends nothing, until a connection attaches again |
| A command prints only beyond the first 32 KiB of its streams while no page is attached | The model's `idleMs` counts from its latest output, within 2 seconds |
| A model switch while a turn runs | The request in flight keeps its model; the turn's next request carries the new model, effort and tier |
| A model switch while an edit is being prepared | The switch is not refused; the replacement turn's first request carries the model it was prepared with, and its next request the new one |
| A client stops reading | The connection closes as lagging; a reconnect adopts the running tree and its turn completes |
| Frames of an open | The handshake order above; patch revisions increase by one; a stale patch after a reset is ignored; a gap triggers `sync_transcript` |
| Scripted tool events | Each of the five tools renders with its own component and its `description` title; updates replace the block in place; an unknown tool name renders as a generic card |
| Tool calls | Input refusals, the repeat guard, preview budgets, handle release and binary stdout verdicts match [Tools](#tools) |
| A stored conversation with images is opened by two pages, and one asks for the transcript again after a gap | No blob is put: every frame carries the references its rows hold |
| A restored conversation with images before and after its last `compaction_boundary` runs a turn of two requests | The first request reads the blob of each replayed medium once and none from before the boundary; the second reads none; both carry the replayed media's bytes |
| A tool's medium cannot be stored | Its result holds `[<kind> not stored: <reason>]`, the model receives that text, and the turn goes on |
| A replayed medium's blob is missing | The model receives `[missing <kind> blob <ref>]` in its place, in every request of the live tree, and the turn goes on |
| One scripted conversation with tools, images, thinking, a steer, a subagent's result and a yield, for each provider | Each request's body begins with the previous request's body, byte for byte apart from the Anthropic cache marks, which the vendor does not count as content; each exception of [The rule](../providers/providers.md#the-rule) changes only what that rule names |
| A switch to a model that does not accept video | Every request to it carries the history's videos as the same text |
| A video whose base64 is over half of a model's request body limit | A tool result does not attach it, and replay sends one already in the history as the same text in every request to that model |
| An image over 2,000 px enters from a tool result and from an upload | It enters fitted, every later request carries the same bytes, and the original stays on the Host |
