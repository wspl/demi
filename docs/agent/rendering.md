# Rendering

The boundary between an agent's transcript and what the page draws: which blocks become which rows, work groups, calls being written, waiting for the model, and tool descriptions; and the frame protocol that carries a conversation to the page and the page's actions back.

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

A renderer uses the `tool_call` fields as [Transcript](transcript.md#transcript) defines
them: `toolName` selects the rendering, `input` is JSON text the renderer
parses, `status` and `output` give the result, and `view` enriches the display
with the command's own output and status. A shell call's `view` also holds
the lists the conversation shows of changed files, which the backend computes
as it stores the block and never changes: `files`, the call's own; `runFiles`,
its run's up to it; `requestFiles`, its request's up to it
([What the conversation shows](../execution/edit-tracking.md#what-the-conversation-shows)).
A page shows the latest call's lists and counts nothing itself. In light form,
only the latest call of each run carries `runFiles`, and only the latest of
each request `requestFiles`, since a page uses no other.

A block the page read from a page of the transcript is in its light form,
without what only its open row shows ([Light form](../product/web-api-conversations.md#light-form)).
Its folded row renders from what it carries, the same as the block whole;
opening the row reads the block whole and shows it in place, with the
row's loading state until it arrives
([Transcript windows](../product/web-application.md#transcript-windows)).

The media a result carries are parts of its `output`, held by
reference ([Media](transcript.md#media)); the view never holds them. Each tool's rendering
shows them with the call, and a medium that is gone where it was
([Media a tool returned](../product/file-previews.md#media-a-tool-returned)).

Live frames add to the transcript; they do not replace it:

- `transcript_reset` and `transcript_patch` are the primary input.
- `shell_output` adds a command's live output and status
  (`crates/agent-server/src/server/tree/live.rs`). While the `shell` call that started the
  command runs, the page shows them under that call, which the frame names
  (`toolUseId`, in the subagent's transcript when `subagentId` is set). Once
  the call has returned, a command that still runs is one of the
  conversation's running commands: the dock's Running chip counts it, its
  panel shows it in a tab titled by the call's `description`, which the
  frame carries as `title` (cut at the end, whole in its tooltip), whether
  or not the page holds the call, and its output keeps coming there, while
  the call keeps the view its result stored. The panel's terminal opens with
  the script as a terminal shows what was typed: `$ ` and the first line, a
  continuation line `> ` for each further line, wrapped at the terminal's
  width, the prompt muted and the script highlighted as Bash, in the colors all code shows in
  ([Code](../product/file-previews.md#code)), then the output; the line is the page's, not part of the command's output. A page adds to what it shows
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
commands, until its end arrives. A command that has run more than 3 seconds
wears how long, counting up, as a grey tag in that place, *1m 32s*, since its
title is the agent's own words and takes no time; a shorter one shows
none, so a quick command does not flash a tag. The tag goes when the command
ends: a succeeded row shows nothing, a failed or stopped one its tag. The expanded row shows the script highlighted as Bash, in the colors all code shows in
([Code](../product/file-previews.md#code)), and says above the output how long the command has run or ran, *Running for
1m 28s*, then *Exited with 0 after 2m 13s*, *Failed with exit code 1 after 47s*
or *Stopped after 5s*; a command that ended within a second says no time,
*Exited with 0*. Component structure, expansion, icons, typography, motion and the
presentation of changed files are shown in the gallery, not here.

### Work groups

An agent often takes many steps in a row: it thinks, runs a command, thinks
briefly, runs three more. Listed one per row, such a run pushes the answer
off the screen. The transcript shows each run of consecutive steps as one
row, as Cursor and Claude's own apps do. For example, the user asks to fix
failing tests, and the agent checks a directory, runs the tests, sets a
script's execute bit, patches a status file and runs the suite again, with
thinking between. While it works, the row shows what is done above what runs
now, *Ran 3 commands* over the fourth command, so the reader sees the work
behind the newest step; once the answer comes, the row reads *Ran 5
commands*, and a click opens it to every step.

- Which blocks make a run the backend decides, once: each block of a run
  carries `run`, the id of the run's first block, set as the block is
  stored, so a page groups by it and judges nothing; a run's start depends
  only on the blocks before it, so `run` never changes. The calls being
  written, which are not blocks, join the run at the transcript's end on the
  page.
- A step is a `thinking` or `redacted_thinking` block, a `tool_call` block, a call being written
  ([Calls being written](#calls-being-written)), a command's notice (a
  `wakeup` that shows, Command reports (`crates/agent-session/src/reports.rs`)), or an agent's
  receipt (an `agent_message` from another agent: an update or a child's
  end). A run is the
  steps between two other blocks. A `resume` block ends no run, as it shows
  nothing: the work after a Continue, a Resume, or the system's own resume
  after a backend restart joins the run before it, which shows its steps as
  one. For example, a deploy command is lost when the backend restarts, the
  turn resumes, and the agent runs four more commands: one row, *Ran 5
  commands*. After Continue, the *Stopped* row rolls into *Requesting*, and
  when the first step arrives it joins the run above and the Requesting row
  gives way to it. What ends a run is what the reader reads:
  the agent's text, never folded, and the user's own words and decisions,
  never folded either: a message, a steer, and the receipts of the user's
  permission decision or move. For example, the agent runs three commands,
  a command it started earlier reports its end, a child reports that it
  finished, and the agent runs two more: one row, *Ran 5 commands*, which
  opens to the five calls, the notice and the receipt in their order.
- Thinking that streams stays one row, *Thinking*, folded as any step, and
  the reader opens it to read along; it never opens by itself, since folding
  itself when it ends would shrink the list with nothing the reader did.
- Thinking blocks next to each other, with no other step between them, show
  as one step: one *Thought for 5s* whose time is theirs together and whose
  text is theirs in order, a paragraph apart, as Claude's and ChatGPT's apps
  show a model's thinking as one. A model may write several thinking blocks
  in one response, Claude between its tool calls and sometimes twice in a
  row; each stays its own block, with its signature, for replay, and only the
  page joins them. Thinking on each side of a call stays two steps, since the
  call between them is what the reader follows.
- Thinking without text, empty or redacted, shows only while it is the newest block of a
  running turn, as what the agent does now; once anything follows it or the
  turn ends, it has nothing to show and goes, from the transcript and from
  the steps of an opened row.
- **Only a step that has ended joins its run.** A call joins its run once
  it returns, one whose command goes on in the background included, and
  thinking once it is written; what is still running, a call being written
  or waiting for its result, or thinking being written, is a row of its own
  under the run, never a line of it. So at the end of a running turn the
  run is what has happened, and the row under it what happens now:

  ```text
  ls tests/                     2s        one step runs: its own row
  ls tests/                               it returns; the next is written
  bun test auth                 12s       and runs, a row of its own
  Ran 2 commands                          it returns and joins the run;
  Requesting                              the turn waits for the model
  Ran 2 commands                          the next step runs under the
  chmod +x scripts/run.sh                 run, which now holds two
  Ran 3 commands                          it returns: the answer follows
  ```

  The run reads and opens as an ended run does (below), at once, whether
  folded or opened, and a step that joins it never changes whether it is
  open; while it holds one step, it reads and opens as that step. The row
  under it is the step that runs, its face and tag as a lone step's, and
  opens on its own, its command and output, as a lone step does; when that
  step ends it leaves its row and joins the run, and the next step, or
  Requesting while the turn waits for the model, takes the row. Only that
  row shimmers, as what runs; the run is done. A step that runs for more
  than 3 seconds shows for how long, counting up as *Thinking for 9s* does.
  A run says its total time in its sentence, running or ended, as *Thought
  for 5s* does: *Ran 3 commands for 1m 32s*, *Ran 2 commands, checked 1
  command for 40s*, the time last:
  the time its steps took, each from its start to its end, added up, so a
  command the agent left running in the background counts only until its
  call returned, and the time the turn waited for the model between steps
  is not counted. A call's block records when its call returned
  (`endedAt`, set as its result is stored); a thinking block ends where the
  next block starts. Steps of thinking alone read nothing,
  so a run that holds only thinking shows only once a step that says
  something joins it.
- An ended run stays one row when it holds a call and another step, and a
  lone thinking stays its own row; a lone call is its own row. The row reads
  what the run did, each kind of call once, in the order the run first did
  it: *Ran 5 commands* for `shell` calls, *checked 2 commands* for calls
  whose script only runs `demi shell status`, counting the commands they
  looked at; a tool of
  another kind reads *used* and its name. For example, a run that thinks
  and checks command 17 twice reads *Checked 1 command*. Notices and
  receipts add to the row only when the run holds no call, by how each
  ended: *2 commands finished*, *1 command failed*, *1 command stopped*,
  *1 command lost* for one whose end the Host could not tell, *1 agent
  finished*, *1 agent failed*, *1 agent stopped*, *heard from 2
  agents* for updates, joined in the order the run first had them, the first
  word capitalized; a run of thinking and notices alone reads the same. A lone notice or receipt is its own row,
  as a lone call is. The row never reads a bare count of steps,
  which says nothing of what happened. A failed command shows its tag on its
  own row inside, not on the group's. A command still running after its run
  ended, as one the agent left running and ended its turn on, wears its
  time on the ended run's row too, *Ran 3 commands* then the time tag, until
  it ends, so a folded row never hides that something still runs.
- The row of the step that runs shows its tag: the time while its command
  runs; once the step ends, its tag goes with it into the run.
- Folded, the files the run's calls changed show under the row, each once,
  counted from its two ends within the run ([What the conversation
  shows](../execution/edit-tracking.md#what-the-conversation-shows)); open,
  each call shows its own.
- A block brought into view inside a run, as a notice's call or a search
  result, marks its own row: a folded run opens for it and stays open
  (`crates/agent-session/src/reports.rs`). Which runs are open is the
  transcript's, not each row's, so a run the reader opened is still open
  after it scrolls away and back.
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
  provider has opened (Tool call start, A run (`crates/provider-common/src/contract.rs`))
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
  `description`, from the moment its description is written; until then the
  row goes on showing what it showed, Requesting or the step before, so no
  placeholder title flashes. Every tool's input lists `description` first,
  so a model, which writes a call's fields in the schema's order, writes the
  title before a long script: the title shows within the call's first tokens,
  and the row shimmers while the rest is written. A page drops a
  pending call whose `toolUseId` it already holds as a block.
- A call's start completes the text before it
  ([Eligibility](conversation-fork.md#eligibility)).

The wait for the model ends when a call's title is written, not when the call
opens ([Waiting for the model](#waiting-for-the-model)). A vendor that sends
calls only whole has no calls being written: between its text and its call,
the tail row says Requesting, and its wait runs on
([Recovering an unfinished turn](../product/turns.md#recovering-an-unfinished-turn)).

### Waiting for the model

While a node waits for the model's answer, the tail row of its transcript
says Requesting with how long the wait has lasted
([Recovering an unfinished turn](../product/turns.md#recovering-an-unfinished-turn)).
How long is the session's fact, not the page's. For example, a subagent's
request has waited eight minutes when the user first opens its view: the view
says Requesting for 8m, and so does a page reloaded a minute later, where a
clock started by the page would say 0s each time and look like a request that
starts again.

- **When a wait starts.** The moment the node needs the model's next answer:
  when a turn starts (a message, a subagent's brief, a wake by a report or a
  message, Retry, Resume), and after each step or input that the turn sends
  on, before the request is built, since building it can read the Host and
  run a preamble hook. A steer the user sends now, which stops the request
  and sends another with the message, starts it again: for example, a
  request has shown nothing for 74 seconds when the user sends a steer now,
  and Requesting reads 0s from that moment, on every page, the sending one
  included. A failed attempt whose output is dropped starts it
  again. Within a request, the end of the text the model wrote last starts
  it again when nothing else shows yet, as when a call follows whose title
  is not written: the page says Requesting there, and the server holds when
  that began, in the same update as the event that ended the text. The
  agent's retries of the same request continue it
  ([Retries](failures-and-recovery.md#retries)): the clock runs on across
  them.
- **When it ends.** The first output of the model ends it: reasoning, with
  text or without, as soon as its block starts, text, a whole call, or a
  call being written once its title is written
  ([Calls being written](#calls-being-written)). Reasoning whose text the
  vendor withholds, redacted or empty, ends it too: the page shows the
  *Thinking* row for it ([Work groups](#work-groups)), so a model that
  thinks never reads as one that has not answered. A request that ended
  showing nothing does not end it. The end of the turn's action ends it, however it ends. A
  compaction pauses it: the page shows the compaction meanwhile, and the
  wait starts again when the compaction ends.
- **What a page receives.** Each start and end is a `waiting` frame:
  `waitedMs`, how long the node had waited when the frame left, or null when
  it waits for nothing, and `subagentId` for a subagent. A page that opens the
  conversation receives one for each node, the root and each live subagent,
  after that node's transcript. The page counts from its own clock when the frame arrives,
  minus `waitedMs`, and compares no time of the server's with its own, so a
  difference between the two clocks never shows.
- **A message the page sent.** Its wait starts at the send, before the server
  has the message ([Recovering an unfinished turn](../product/turns.md#recovering-an-unfinished-turn)):
  that page counts from the send, the earlier of the two starts.

So whenever the row says Requesting, the server holds the wait's start, but
for the few milliseconds while new input is saved; the page counts from when
the row began to say so until the `waiting` frame arrives.

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
([Pages](../product/web-api-conversations.md#pages)), and asks the stream for the blocks
from its last one on:

```text
client                               server
open { from: 2999, edge: b_2999 } -> attach to the live tree
                <------------------- opened
                <------------------- transcript_reset { start: 2999, blocks, version: { epoch, revision: r } }
                <------------------- phase, queue, pending_steers, pending_calls, waiting
                <------------------- subagent started + subagent_transcript_reset
                                     + pending_calls + waiting,
                                     for each live subagent, depth first
                <------------------- shell_output, for each live command
                <------------------- transcript_patch { revision: r + 1 }
```

### Client frames

| Frame | Meaning | Answer |
| --- | --- | --- |
| `open { from, edge }` | Attach this connection to the conversation's tree, restoring the tree when it is not live, with the model selection the conversation's record holds; `from` and `edge` name the root's blocks the page holds ([Where a reset starts](#where-a-reset-starts)) | `opened`, then the snapshot frames, whose state the page takes whole ([What a reopened socket brings back](../product/web-application.md#what-a-reopened-socket-brings-back)) |
| `send { messageId, content }` | Submit a message ([Input](runtime.md#input)) | Transcript, phase and queue frames |
| `edit_and_send { request }` | Replace a user message and its suffix ([Message editing](message-editing.md)) | `edit_result` at durable acceptance |
| `steer { steerId, content }` | Add input to the running turn | `steer_result` |
| `send_saved { id, turnId, delivery }` | Send a message saved for later as the composer would, `delivery` `message`, `steer` or `queue`, as the message `turnId`, and remove it from the list in the same step ([Messages saved for later](../product/web-api-conversations.md#messages-saved-for-later)). The page keeps it as a message on its way that remembers it came from `id`, across a reload, so Retry sends `send_saved` again with the same `turnId`; a `turnId` the session already took answers as the first did, and the saved message is gone either way | The frames of that delivery; `rejected` with `saved_not_found` when the message is gone and `turnId` was never taken |
| `cancel_pending_steer { steerId }` | Withdraw a pending steer | `remove_result`, after the `pending_steers` list |
| `steer_now { steerId }` | Deliver a pending steer now ([Send now](runtime.md#send-now)) | None; the transcript and the `pending_steers` list |
| `dequeue_message { messageId }` | Remove a queued message | `remove_result`, after `queue` |
| `send_queued_message { messageId }` | Send a queued message now ([Send now](runtime.md#send-now)) | `queue` |
| `clear_message_queue` | Remove every queued message | `queue` |
| `abort` | Stop one thing ([Stop](runtime.md#stop)) | `abort_result`, in request order |
| `abort_subagents`, `abort_subagent { subagentId }` | Stop subagents (`crates/agent-server/src/server/tree/supervisor.rs`) | `subagent` frames |
| `show_subagents { ids }` | The children whose transcripts this connection shows (`crates/agent-server/src/server/tree.rs`) | A `subagent_transcript_reset` for each child it adds |
| `retry`, `resume`, `compact` | Run the action; `compact` takes `node`, the root when absent, so a subagent's context card compacts that child | `rejected` when the session is busy |
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
number (`{ type: "attachment", id }`)
([Files the edit keeps](message-editing.md#files-the-edit-keeps)). A `send`
or `steer` that holds `media` or `attachment` is an invalid frame. No frame
carries file bytes. The backend
resolves uploads and remote files before the session sees the content
([Media by reference](../backend/backend.md#media-by-reference),
[Device files and remote references](../product/web-api-hosts.md#device-files-and-remote-references)),
and the session resolves an edit's kept files from the edited message
([Files the edit keeps](message-editing.md#files-the-edit-keeps)).

`shell_write` and `shell_abort` reach the command through the shells of the
node that runs it, the root or a live subagent, for the conversation's current
Host, with the handle checks of Running shell tools (`crates/agent-tools/src/lib.rs`).

### Server frames

| Frame | Carries |
| --- | --- |
| `opened` | The connection is attached |
| `transcript_reset` | `start`, the blocks from that index to the end, whole, the transcript's `length`, the version `{ epoch, revision }`, and `failures` ([Where a reset starts](#where-a-reset-starts)) |
| `transcript_patch` | Patches, the new revision, and `failures` |
| `phase` | `idle`, `running` or `compacting`, and `subagentId` when it is a live subagent's: a page opening the conversation receives one for the root and each live child, and a child's tail row reads its own, so a child idle beside a running command shows no Requesting row |
| `context_usage` | `node`, the agent whose context it is, and the estimate of that node's next request in `tokens`, the `window` in use and `compactFrom`, the estimate from which `compact` is taken; both null for a model without a window; and `account`, the account the node's requests use under Account rotation (`crates/backend-providers/src/llm/rotation.rs`), null when the entry has none or the node has not chosen one yet. Right after the open handshake when the session can tell it without reading a blob ([Context estimate](compaction.md#context-estimate)) |
| `queue` | The queued messages, each `{ id, content }` |
| `pending_steers` | The complete list of pending steers |
| `steer_result` | The steer id and an `outcome`: `{ status: "accepted" }` or `{ status: "rejected", reason }` |
| `edit_result` | The operation id and an `outcome`: `{ status: "accepted", turnId }` or `{ status: "rejected", reason }` |
| `abort_result` | What was stopped, and whether another `abort` would stop more |
| `shell_output` | A command's live view (`crates/agent-server/src/server/tree/live.rs`): `subagentId` when the command is a subagent's, and its `status`: `running`, `exited` with the `exitCode`, or `aborted`, each with the `commandId`, the `toolUseId` of the `shell` call that started it, its `title` and `script` as its record keeps them from its start, the `tail` and `chars` of the pages' view, and `runningMs` |
| `shell_write_result` | The command id |
| `remove_result` | The steer's or queued message's id and an `outcome`: `removed`, or `gone_on` when the turn had already taken it or the session never held it |
| `pending_calls` | The calls the model is writing, each `{ toolUseId, toolName, description }`, `description` null until written, and `subagentId` when they are a subagent's ([Calls being written](#calls-being-written)) |
| `retry_scheduled` | The attempt, the delay in milliseconds, the code and the diagnostics of a failure being retried ([Retries](failures-and-recovery.md#retries)) |
| `waiting` | `waitedMs`, how long a node had waited for the model's answer when the frame left, or null when it waits for none, and `subagentId` when the node is a subagent: when a wait starts or ends, and on open for each node ([Waiting for the model](#waiting-for-the-model)) |
| `error` | A message, a code and diagnostics: a failed turn, a refused frame, or a failed save; `requestId`, the socket's ([Request IDs and the server log](../backend/backend.md#request-ids-and-the-server-log)) |
| `rejected` | The refused command, the reason's code and its values, and `requestId`, the socket's, as an `error` carries it |
| `subagent`, `subagent_transcript_reset`, `subagent_transcript_patch` | Subagent lifecycle and transcripts (`crates/agent-server/src/server/tree.rs`) |
| `closed` | The connection is detached: its tree was disposed, or it sent `close` while attached to none |
| `heartbeat` | Nothing: the connection sent no other frame for 30 seconds ([Order and delivery](#order-and-delivery)) |
| `pong` | `id`: the page's `ping` it answers, at once and ahead of any other frame waiting ([A dropped connection](../product/web-application.md#liveness-and-reconnection)) |

`failures` is the backend's reading of the error blocks a frame carries,
attached when the frame is sent and never stored
([Failure facts](../backend/backend.md#failure-facts)). Blocks, queued
messages and pending steers hold their media by blob reference, so no frame
carries media bytes ([Media](transcript.md#media)).
Every `shell_output` is a command's live view, the same for every page: it
reports a change of a live command of any node of the tree, or answers an
attach or a `sync_transcript` (`crates/agent-server/src/server/tree/live.rs`). No tool sends
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
  it (`crates/agent-server/src/server/tree.rs`).

The page applies a patch to the blocks it holds. A patch to a block the page
does not hold, between two parts of the transcript it read, changes nothing
the page shows; the page reads that block as it is when it reads its page,
and an `add` or a `truncate` moves the indices of the blocks after it
whether the page holds them or not.

### Order and delivery

- Client frames are strict: an unknown field is refused. The client accepts
  server frames with fields it does not know.
- The backend sends the frames its outbox holds at once in one WebSocket
  message: one frame as a JSON object, several as a JSON array in order, as
  JSON-RPC batches do. The page applies a message's frames in order and draws
  once, so frames ready together, such as the handshake's, are one change.
- A response that asks for no tool holds the frames of its turn's end until
  the turn goes on, by input, the run's next response (as when Claude Code
  answers two messages of one run in turns of their own), a compaction or a
  provider error the retry decides on, or until the conversation goes idle after its closing save; the
  last patch and `phase: idle` then go in one message. The hold waits only for
  that save, never a timer, and a Stop releases it before `abort_result`.
- A frame that does not match its schema is answered with an `error` whose code
  is `invalid_frame`, and the connection stays open. A message that is not JSON,
  or a binary one, closes the connection with close code 1007 `not_json`.
- The backend handles one connection's frames one at a time, in arrival order.
  A frame waits for the conversation's admission and is refused while the
  conversation is archived
  ([Sidebar mutations and read state](../product/web-api-conversations.md#sidebar-mutations-and-read-state)).
  A frame whose handling waits, such as a send whose uploads are being written
  to the Host or an edit waiting for its durable acceptance, delays the frames
  behind it.
- Every frame for one attachment, whether a reply or an event, goes through
  one outbox in causal order. The outbox holds at most 4,096 frames. When it is
  full, the server closes that connection with close code 4001 `lagged`, and
  the tree's other attachments go on as before; the client reconnects and
  adopts the running tree. A backend that shuts down closes its conversation
  sockets with 1001 `backend_closing` before it disposes the trees. A
  session that is signed out, logs out or expires closes its conversation
  sockets with 4002 `session_ended`, and the page goes to sign-in.
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
  `opened` and the handshake's last frame, so the snapshot frames agree with
  each other.
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

- A page that opens a conversation without a copy of it shows it in two
  steps ([Opening a conversation](../product/web-application.md#backend-communication)):
  the transcript with its agents once both are read, then the phase, the
  queue, the pending steers, the wait and the running commands together
  once its handshake is applied, so a reload never draws a moment without
  the steers or with a clock at 0s. A conversation that opens no session,
  archived or without a usable model, stops at the first.
- A tree has any number of attached connections. `open` attaches its
  connection without detaching another, and the connection receives its own
  handshake ([Order and delivery](#order-and-delivery)), then every event of
  the tree.
- An event goes to every attached connection: the transcript, phase, queue,
  pending-steer, retry and subagent frames, the `error` of a failed turn, save
  or subagent lifecycle, and every `shell_output` of a live command's change
  (`crates/agent-server/src/server/tree/live.rs`). A reply goes only to the
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
  ([the conversation's model settings](../product/web-api-conversations.md#sidebar-mutations-and-read-state)).
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
  ([Messages and the queue](runtime.md#messages-and-the-queue)).
- An edit and a send: a send that arrives while the edit is being prepared is
  refused, and an edit that arrives while a send's turn runs or waits is
  refused ([Admission](message-editing.md#admission)). An edit against a
  transcript that another connection has changed since is stale
  ([Commit and idempotency](message-editing.md#commit-and-idempotency)).
- Two `abort` frames stop one thing each, in the order they arrive
  ([Stop](runtime.md#stop)): the second can stop a queued message, and its
  `abort_result` says what it stopped.
- No frame changes the model selection: the conversation's model settings
  change by a conversation patch, which the backend applies one at a time
  ([the conversation's model settings](../product/web-api-conversations.md#sidebar-mutations-and-read-state)).
