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
  actions, steers, agent messages, model changes and new yield wakeups.
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
  |   request the provider, retrying transient failures
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
([Retries](failures-and-recovery.md#retries)); compaction inside a turn is
described in [Compaction](compaction.md#compaction).

While an action runs, its stage is one of: preparing (before the first
request), streaming from the provider, running tools, compacting, or
finalizing (saving at the end). Clients see the phase `idle`, `running` or
`compacting`. The phase, the visible queue and whether the session has settled
are derived from the one status and the waiting actions; no second flag records
them.

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

`set_provider` changes the model selection:

- By default, the switch waits for the next action. With `apply: immediate`,
  it also lands inside a running turn at the next continuation boundary, so
  the next request uses the new model.
- When the history is over the new model's compaction threshold, the session
  first compacts with the current model and provider, then switches
  ([Compaction](compaction.md#compaction)). An immediate switch that compacted
  appends a `resume` block.
- A switch to another provider builds a new provider runtime. The replaced
  runtime is closed once no run uses it, and so is the runtime of a pending
  switch that a later switch replaced.

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

- The model and the user's page each keep their own place in a command's
  output. A result shows the output since the model's last look at the
  command, and a `shell_output` frame the output since the page's last look
  ([Server frames](#server-frames)), so neither's read changes what the other
  sees next. For example, the page reads a running command's new output; the
  model's next `shell_status` still shows all of it.
- A result gives the command's status and exit code, its handle and timings
  when the handle matters, a preview of the output, and a hint for the next
  step.
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
  the model-media table, the model accepts that type, and it fits the cap for
  its kind: 4 MiB for an image, 16 MiB for a video. Otherwise the result says
  why nothing was attached and where the raw bytes remain readable.

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
| `resume` | A turn continuing after a cut: `resume`, compaction inside a turn, or an immediate model switch that compacted | A user message: "Continue from where you left off." | No |
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

The model receives the blocks from the last `compaction_boundary` onward,
each as the table says. A long text is cut in the middle by the replay bound
([Text bounds](compaction.md#text-bounds)). Signed thinking and redacted data
are replayed whole, because the vendor verifies them as they were sent. Media
stored as blob references is loaded back before the request
([Tree store](#tree-store)). A tool call's input is replayed as the JSON value
the provider supplied, or as text when it is not valid JSON.

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

### Patches and versions

Every change a session makes to its transcript is recorded as patches, each
naming the index of the block it touches:

| `op` | Fields | Meaning |
| --- | --- | --- |
| `add` | `index`, `value` | Insert a block at the index |
| `remove` | `index` | Remove the block at the index |
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
- `shell_output` adds live output and status to a running shell command.
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
([Web API](../product/web-api.md)). The connection belongs to that one
conversation: the backend supplies the session id and the working directory
from the conversation's target, and the client never sends them. The types of
every frame are Rust types, and the browser validates frames with the schemas
generated from them ([Generated TypeScript](../architecture/contracts.md#generated-typescript)).
`AgentClient`, in `@demicodes/agent-client`, is the browser's client of this
protocol.

For example, a page opens a conversation whose root is running:

```text
client                               server
open { model } --------------------> attach to the live tree
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
| `open { model }` | Attach this connection to the conversation's tree, restoring the tree when it is not live, and select the model | `opened`, then the snapshot frames |
| `send { messageId, content }` | Submit a message ([Input](#input)) | Transcript, phase and queue frames |
| `edit_and_send { request }` | Replace a user message and its suffix ([Message editing](message-editing.md)) | `edit_result` at durable acceptance |
| `steer { steerId, content }` | Add input to the running turn | `steer_result` |
| `cancel_pending_steer { steerId }` | Withdraw a pending steer | None; the `pending_steers` list |
| `dequeue_message`, `send_queued_message { messageId }` | Remove a queued message; run one next | `queue` |
| `steer_queued_message { messageId, steerId }` | Turn a queued message into a steer | `steer_result` |
| `clear_message_queue` | Remove every queued message | `queue` |
| `set_provider { model, apply? }` | Change the model selection ([Model switch](#model-switch)) | None, or `error` |
| `abort` | Stop one thing ([Stop](#stop)) | `abort_result`, in request order |
| `abort_subagents`, `abort_subagent { subagentId }` | Stop subagents ([Abort](subagents.md#abort)) | `subagent` frames |
| `retry`, `resume`, `compact` | Run the action | `rejected` when the session is busy |
| `shell_write { commandId, stdin }` | Write stdin to a running command | `shell_output`, then `shell_write_result` |
| `shell_abort { commandId }` | Stop a running command | `shell_output` |
| `sync_transcript` | Ask for a fresh transcript | `transcript_reset`, `shell_output` for each live command, the subagent replay |
| `close` | Dispose the tree | `closed` |

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
[Device files and remote references](../product/web-api.md#device-files-and-remote-references)).

`shell_write` and `shell_abort` reach the command through the root's shell
environment for the conversation's current Host, with the handle checks of
[Running shell tools](#running-shell-tools).

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
| `shell_output` | A command's status (`running`, `exited` or `aborted`) with its shell and command ids and its bounded output views; binary stdout is described by size and truncation, never sent as bytes |
| `shell_write_result` | The command id |
| `retry_scheduled` | The attempt, the delay in milliseconds, the code and the diagnostics of a failure being retried ([Retries](failures-and-recovery.md#retries)) |
| `error` | A message, a code and diagnostics: a failed turn, a refused frame, or a failed save |
| `rejected` | The refused command and the reason |
| `subagent`, `subagent_transcript_reset`, `subagent_transcript_patch` | Subagent lifecycle and transcripts ([Protocol](subagents.md#protocol)) |
| `closed` | The connection is detached |

`failures` is the backend's reading of the error blocks a frame carries,
attached when the frame is sent and never stored
([Failure facts](../backend/backend.md#failure-facts)). Media in outgoing blocks
travels by blob reference ([Media by reference](../backend/backend.md#media-by-reference)).
A running shell tool's status reaches the client as `shell_output`; a
`shell_status` call sends none. Every `shell_output` is the page's own view,
with the output since the page last looked at the command, whether it answers
a client's `shell_write` or `shell_abort`, follows an attach or a
`sync_transcript`, or reports a running tool; it moves only the page's place,
not the model's ([Results and previews](#results-and-previews)). Child
sessions send only their transcript frames. A live command is one the
transcript last shows running that the root's shell environment still has;
its `shell_output` after a transcript reset carries its current status, so a
command that ended while no client watched shows as ended.

### Order and delivery

- Client frames are strict: an unknown field is refused. The client accepts
  server frames with fields it does not know.
- A frame that does not match its schema is answered with an `error` whose code
  is `invalid_frame`, and the connection stays open. A message that is not JSON,
  or a binary one, closes the connection with close code 1007 `not_json`.
- The backend handles one connection's frames one at a time, in arrival order.
  A frame waits for the conversation's admission and is refused while the
  conversation is archived
  ([Sidebar mutations, read state and page synchronization](../product/web-api.md#sidebar-mutations-read-state-and-page-synchronization)).
  A frame whose handling waits, such as a send whose uploads are being written
  to the Host or an edit waiting for its durable acceptance, delays the frames
  behind it.
- Every frame for one attachment, whether a reply or an event, goes through
  one outbox in causal order. The outbox holds at most 4,096 frames. When it is
  full, the server closes the connection with close code 4001 `lagged`; the
  client reconnects and adopts the running tree. A backend that shuts down
  closes its conversation sockets with 1001 `backend_closing` before it
  disposes the trees.
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

- A connection that closes only detaches. The tree's turns keep running, and
  the next `open` adopts the same live tree. Two concurrent opens of one
  conversation share one tree.
- Opening a conversation that another connection is attached to takes it over:
  the other connection receives `closed` and is detached. It can send `open`
  again to take the conversation back.
- `open` aligns the tree's model with the model it names, from the root's next
  turn on; a restored root keeps its checkpoint's model until then. When that
  model belongs to another provider, the new runtime is built before the
  connection attaches, and a failure leaves the connection unattached.
- `close` disposes the whole tree, then sends `closed`, even when nothing was
  attached.
- A tree that has been detached and quiescent (no action running or waiting,
  no live subagent, no scheduled wakeup) for 10 minutes is disposed; an `open`
  or new activity within those 10 minutes keeps it live. The next `open`
  restores it from the store exactly as it was saved.

## Tree store

The tree store is the persistence contract between the agent and a
conversation's database. One store holds one conversation's tree: every node
with its parent link and its checkpoint. The backend realizes it over the
conversation's database
([Conversation state and transactions](../backend/storage.md#conversation-state-and-transactions)).
Tests use an in-memory store that meets the same contract.

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
  action happens after the phase is idle, so a checkpoint that says an action
  was running is one the process died in.
- A save is due when the transcript, command state, edit receipts, the queue,
  the waiting agent messages or the model selection changed. The phase alone
  never makes a save due.
- A scheduled save that fails is reported as an `error` frame, and its rows are
  saved with the next change. When the save at the end of an action fails, the
  action fails.
- A history rewrite is saved before it is published: the store commits the
  retained rows with the command state of the cut, then the session adopts them
  and publishes one `replace` patch.

### Media

- Before a save, the store moves each block's inline media into the
  conversation owner's blob namespace and saves references instead. A blob is
  published before the row that references it
  ([Attachment and transcript media](../backend/storage.md#attachment-and-transcript-media)).
- Loading a session for inference puts the bytes back. A reference whose blob
  is missing becomes the text `[missing <kind> blob <ref>]`, so the turn goes
  on; a malformed reference is a decode error.
- The agent defines this mapping between inline bytes and references; where
  the bytes go is the store's decision. The agent's server never sees a blob
  store.

### Restoring

- The store decodes and validates every row it reads, and corrupt data stops
  the restore ([Storage](../backend/storage.md)).
- The session adds its own checks: the checkpoint's harness is the node's, the
  waiting agent messages have unique ids that are not already in the
  transcript, each is addressed to this node, and edit operation ids are
  unique.
- The session then completes interrupted tool calls and hands back its queue
  as [Dispose and restore](#dispose-and-restore) describes.

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
| A message is queued while a tool runs | The queue is saved without any transcript change |
| Two opens of one conversation at once | One live tree |
| A client stops reading | The connection closes as lagging; a reconnect adopts the running tree and its turn completes |
| Frames of an open | The handshake order above; patch revisions increase by one; a stale patch after a reset is ignored; a gap triggers `sync_transcript` |
| Scripted tool events | Each of the five tools renders with its own component and its `description` title; updates replace the block in place; an unknown tool name renders as a generic card |
| Tool calls | Input refusals, the repeat guard, preview budgets, handle release and binary stdout verdicts match [Tools](#tools) |
