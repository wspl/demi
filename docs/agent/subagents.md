# Subagents

A subagent is a child agent session that another session starts with a
`demi agent` command. For example, a conversation's root session asked to
refactor authentication spawns one child to find every call site and another to
update the documentation, continues its own work, and ends its turn. Each
child's result wakes the root when that child finishes.

Sessions form a tree of any depth, rooted at the conversation's root session.
Every node of the tree, the root included, is the same kind of session: built by
the same assembly, stored under the same contract ([Runtime](#runtime),
[Persistence](#persistence)), carrying the same `demi agent` command group, and
supervising its own children. The model-facing tool surface stays the five
standard tools ([Tools](runtime.md#tools)). The browser receives the whole tree
on the conversation's socket, beside the root's own frames
([Protocol](#protocol)).

A subagent is not a session copy. The session copy that compaction uses
([Session copy](compaction.md#session-copy)) copies part of a session's history
to write a summary; a subagent starts with an empty transcript.

## Why

Parent context is expensive. Parallel exploration and focused work need a fresh
session, the same Host, and a result that returns to the parent without dumping
the child's tool history into the parent's inference transcript.

The shell runs the `demi agent` commands. Creation, communication, and abort
are short command calls; the parent's supervisor runs each child independently.

Depth is not capped. A child delegating a slice of its own task spawns exactly
like the root does; there is no per-depth command stripping. What bounds the
tree is fan-out (the live-children limit in [Runtime](#runtime)) and the real
turns each spawn costs. A spawner can still forbid one specific child from
delegating further, with `--no-subagents` at spawn or a profile that forbids
spawning. That is an explicit per-child restriction, never derived from depth:
the child loses `spawn`, `abort`, and `resume`, and keeps `send`, `list`, and
`show`.

## Topology and the agent directory

Each tree keeps one agent directory: a flat registry of every live session in
the tree, keyed by session ID. Each entry carries the parent session ID,
description, profile, and phase. Spawn registers an entry; close unregisters
it.

The directory is the sole basis for cross-tree addressing. `send` and `show`
resolve their target ID against it: any live agent in the tree is addressable,
regardless of the sender's position. There are no routing rules along the tree.

Authority is split by verb, not by depth:

- **Lifecycle** (`spawn`, `abort`, `resume`): only on your own direct children.
  Whoever spawns an agent owns its life; nobody else stops or revives it.
- **Communication** (`send`) and **reads** (`show`, `list`): any live agent in
  the tree.

## Model-facing surface

The model still sees:

```text
shell_exec
shell_status
shell_write
shell_abort
yield
```

The agent server grafts a `demi agent` group into every node's command set; the
root and every subagent receive the same group, except that a spawn-restricted
node's group lacks `spawn`, `abort`, and `resume`. When the harness declares a
`demi` group, `agent` joins it, replacing any `agent` child the harness
declared. Otherwise the server adds a `demi` root, "Demi agent runtime
commands.", that contains only `agent`.

```text
demi agent spawn [--request-id <id>] [--profile <name>] [--description <title>] [--no-subagents] < task-brief.txt
demi agent abort <id>
demi agent resume <id> [--request-id <id>] < message.txt
demi agent send <id|parent> < message.txt
demi agent show <id>
demi agent list
```

`demi agent spawn` reads a task brief from stdin, creates a persisted child, and
returns immediately. Stdout is `subagentId: <id>`; `--json` returns
`{ "subagentId": "<id>" }`. Success confirms creation, not completion. The
parent's supervisor owns the child after creation, independently of the
invoking shell job, its stdout pipe, and its cancellation
([Creation command ownership](#creation-command-ownership)). `resume` has the
same acceptance-only response after durably queuing the next round's message.

Use `agent send` to communicate and `agent abort` to stop a child. Spawn's
`shell_status` only describes the completed creation command. A parent can
start several children one after another and continue its work; ending its turn
lets completion receipts wake the parent when the results are available. No
polling or timed yields are required.

Both start commands accept `--request-id <id>`. A caller that may retry an
uncertain response supplies this ID on the first attempt and reuses it with
identical arguments. The owning node's command storage keeps an immutable
reservation at `agent.start.<id>`: the normalized arguments, the child ID, and
the round's start time. The reservation is committed before the child is
created or reopened.

- A retry finishes an uncommitted start, or returns the existing child without
  starting another round.
- Different arguments for the same request ID fail.
- A request without an explicit ID gets a new one.
- A resume reservation superseded by a later round fails instead of replaying
  an older message.

Starts are serialized within the owning supervisor, including concurrent
command calls. Resume requires the previous completion to be saved in the
parent's checkpoint before it replaces the archived round. The parent's command
state history retains these reservations
([Command state history](command-state-history.md)).

Prompts and `send` and `resume` messages are read only from stdin: a quoted
heredoc, a pipe, or input redirection. They have no positional or option form.
An empty message fails. `--profile` names a [profile](#profiles);
`--no-subagents` forbids the child from delegating. There is no `--model`
option: the model and the provider runtime come from the profile or the
parent.

With `--json`, `abort`, `send`, `show`, and `list` return `{ id, aborted }`,
`{ id, accepted }`, `{ agent }`, and `{ tree }`.

### Command help

How to write the child's task brief lives only on the spawn command's `prompt`
field, which the command reads from stdin. That text is the child's first user
message. Help renders a quoted heredoc usage template from the declaration
([Help](../execution/commands.md#help)). Demi does not reject a short prompt.

The field description follows the same four beats as the compaction summary
instruction ([Keeping the cache prefix](compaction.md#keeping-the-cache-prefix)):
job, stance, inventory, and output. Compaction has the history and must not
obey it. A child has no history, so the parent must put the continuation facts
into this argument:

> The child's first user message and only task brief. The child starts with an
> empty transcript and cannot see this conversation: do not refer to prior
> turns, and do not paste this conversation or the product user's message
> unchanged. Include the goal for this child, applicable decisions and
> constraints, whether to edit or only report, how to verify, and every
> concrete identifier it needs (paths, ids, error text, commands already tried
> and their key results). State the exact shape of the last assistant text it
> should return.

`--description` is a short UI title that tells concurrent children apart. The
`--profile` help lists the configured profile names.

## Communication

Agents in a tree communicate through agent messages. An agent message is input
that one agent sends another: an explicit `demi agent send`, or a child's
automatic completion receipt.

For example, a child finishes while its parent executes a tool. The receipt
waits in the parent's pending input. After the tool completes, the parent's
next continuation boundary writes one `agent_message` block into its
transcript, and the parent continues the same turn with the child's result in
context. The browser renders a collapsed receipt row from that block
([Product rendering](#product-rendering)).

### Delivery behavior and transcript type

Two independent dimensions describe an agent message:

- **Delivery** is steering: the input enters an active turn at its next
  continuation boundary ([Input](runtime.md#input)). No provider receives input
  in the middle of a request.
- **Representation** is the `agent_message` block: input from another agent,
  with its source and event details. Human steering materializes as `steer`
  blocks.

The session admits agent messages into the same pending-input queue as human
steers, and wakes an idle recipient with an internal continuation. The pending
record carries the structured agent source, and the session materializes it as
an `agent_message` block. There is no separate inbox, scheduler, or provider
delivery loop for agent messages.

### Message identity

An agent message is structured input from another agent, not a human message
and not the receiving agent's assistant output. The supervisor supplies the
sender identity from the invoking node; a model cannot impersonate another
sender by writing an identifier into the message body.

| Field | Meaning |
| --- | --- |
| `id` | The stable message ID |
| `sender` | The sender's node ID; its description (`root session` for the root); and its round, the sender node's persisted spawn time in milliseconds |
| `recipientId` | The recipient's node ID |
| `timestamp` | When the message was sent; for a completion, when the child closed |
| `content` | The body |
| `event` | `message`, an explicit communication between live agents; or `completion`, a supervisor receipt with the `outcome` `completed`, `failed`, or `aborted` |

A completion carries the child's result for `completed`, and otherwise its
failure text, which can be empty. Its ID is `subagent:<child id>:<round>`, so
it identifies exactly one execution round of one child; reopening a child
starts a different round. An explicit message must not be empty.

The source is structured data, independent of presentation text: neither replay
nor the browser parses a bracketed text prefix to find it. The recipient's
session validates each message when it admits it
([Validation at entry](../architecture/contracts.md#validation-at-entry)).

### One communication operation

The model-facing operation is `demi agent send <id|parent>`. Its meaning is
"deliver this information to that agent." The runtime decides when to deliver;
the sending model does not choose between a mailbox and a steer. There is no
separate agent-to-agent steer verb. Human steering remains a distinct action.

`send` returns after durable acceptance, not after inference or an answer. It
never waits for another agent to finish, so reciprocal messages cannot block
each other's commands. A send fails when it names the sender itself, `parent`
from the root, or an agent that is archived or closing; only the parent can
reopen an archived child, through `resume`.

Automatic completion receipts use the same admission. The child returns its
final answer once; the supervisor delivers it. The child's instructions ask for
explicit messages only for useful interim information, questions, or blockers,
never for a duplicate of the final result.

### Delivery and scheduling

The session admits explicit messages and completion receipts through one entry.
While the recipient runs, a message waits in its pending input and enters the
transcript at the next continuation boundary. While the recipient is idle, the
session opens an internal continuation. Agent inputs never appear in the human
message queue or in the list of pending steers the browser shows.

| Recipient state | Delivery |
| --- | --- |
| Preparing a request, streaming, or executing tools | Consumed at the next continuation boundary, before more model output is requested. |
| Compacting | Kept outside the compacted prefix and consumed before inference resumes after compaction. |
| Finalizing | Kept; pending input is checked again before the session commits idle or the child closes. |
| Naturally idle | The available batch is consumed by one internal continuation, without a human user bubble. |
| Stopped by the user | Kept without automatically resuming the stopped work; the user's next action can consume it. |
| Archived | Explicit sends are refused; a completion receipt stays durable at its owning supervisor until it can be delivered. |

Messages do not cancel an executing tool or restart the current turn. A
boundary can come later than the arrival, so the browser must not claim that
the model has read a receipt merely because the runtime accepted it.

At a boundary, the session writes pending messages into the transcript in
admission order, before the next inference request. Distinct messages keep
distinct IDs and content. An idle recipient's continuation incorporates every
message available at that moment instead of starting one turn per message;
messages that arrive after that boundary belong to the next opportunity.

The session orders admission with finalization, so a race between finishing and
admission cannot lose a message. Pending agent input counts in the quiescence
check and keeps a child open ([Result](#result)).

The recipient treats receipts as context for its active task. It does not owe a
separate user-facing acknowledgement for each message. If nothing else can
advance until a child returns, it ends its turn and relies on delivery; it does
not poll or schedule short timed wakeups.

### Durable ownership and replay

The recipient's checkpoint keeps each pending agent input: the turn it targets,
the model current at admission, and the message. The message ID and body live
only in the message. Admission validates the message and saves the checkpoint
before `send` returns. Materialization replaces the pending record with the
transcript block in the same checkpoint. There is no second mutable "delivered"
flag on the block and no separate message store.

A completion is acknowledged for its source round only when the parent has
durably accepted responsibility: in the same transaction that saves the pending
input or its materialized block ([Persistence](#persistence)). Restore retries
unacknowledged rounds with their original IDs. Admission deduplicates against
pending input IDs and materialized `agent_message` IDs: the same ID with the
same content counts as already admitted, and the same ID with different content
is refused. Replaying a committed message neither creates a second receipt nor
schedules a second wakeup.

Provider interruptions follow the ordinary recovery rules
([Failures and recovery](failures-and-recovery.md)). A message enters the
transcript exactly once; inference across a failed provider connection is not
exactly once.

Replay sends an `agent_message` block to the provider as steering input with an
explicit source envelope: two fixed lines saying that this is agent-originated
context, subject to the real user's task and constraints, that needs no
separate acknowledgement, followed by the message as JSON. A provider may need
a user-role item to carry it; that wire role does not make the message
human-authored in the transcript or the browser. The message is never replayed
as the recipient's own assistant answer.

Editing, Fork, compaction, and restore read the same structured contract.
Message editing and Fork are not offered on receipts
([Message editing](message-editing.md)). Compaction keeps necessary receipt
information as task context; it does not turn source text into a human
instruction. Historical receipts do not replay notification side effects or
resurrect the sending child.

## Result

A child is quiescent when it has no running or queued action, no unread agent
message, no scheduled yield wakeup, and no live child of its own. The
supervisor closes a quiescent child with its last assistant text, cut at a
character boundary to at most 32 KiB of UTF-8. A child whose action fails
terminally closes as `error` with the failure text. Checking quiescence and
deciding to close are one step: a message accepted before it keeps the child
open, and a send after it is refused because the child is closing.

The supervisor saves the result before it delivers a completion message to the
parent. A naturally idle parent wakes; a busy parent incorporates the receipt
at its next continuation boundary. The creation command carries no completion
result. A child with a scheduled wakeup or live descendants stays live.

Each execution round has a distinct completion message ID that contains the
child ID and the round, the child's persisted spawn time in milliseconds.
Resume chooses a round strictly newer than the previous one: the current time,
or the previous round plus one when the clock has not moved past it. A parent
checkpoint marks only the matching round delivered; an older completion cannot
acknowledge a newer round. Restore retries an undelivered completion with the
same message ID, and admission deduplicates it against pending inputs and
`agent_message` blocks.

A child that needs a decision ends with its question as the result; its parent
runs `demi agent resume <id>` with the answer on stdin to start the next round
on the preserved transcript.

## Observe

The parent model does not receive `subagent_transcript_*` frames. Those frames
are for the browser. Pushing child tool history into the parent's inference
transcript is the failure this design exists to avoid.

A stuck child also cannot be asked to report on itself: a steer waits until the
child reaches a boundary, so a hung tool or provider stream never answers.
Observation is a supervisor read. It does not inject into the target transcript
and does not wait on it.

Two snapshot reads:

| Command | Answers | Does not answer |
| --- | --- | --- |
| `demi agent list` | The whole live tree, plus archived children | One agent's recent work |
| `demi agent show <id>` | A bounded snapshot of one live agent | The full transcript, tool outputs, or thinking |

Durations read like `45s`, `4m`, `1m5s`, `2h`, or `1h3m`, rounded to the
nearest second.

### `demi agent list`

Renders the session tree of this conversation from the root down, marking the
caller's own position. Live agents show phase, elapsed and last-event ages,
profile, description, execution, and activity, and are ordered by spawn time,
and by the order they joined the tree when those are equal.
Each node's archived children follow its live ones, newest first, with their
closed phase and age:

```text
● c1  (root session)
├─● ag_x7f9k2  running  up 4m  last-event 8s ago  profile=(inherit)  "refactor auth"  execution=provider_streaming  activity=streaming
│ ├─● ag_m3n8  running  up 1m  last-event 5s ago  profile=(inherit)  "search call sites"  execution=tool_executing  activity=grep call sites ← you
│ └─○ ag_q5w2  archived (completed 3m ago)  "update tests"
└─● ag_z4r6t0  running  up 2m  last-event 40s ago  profile=(inherit)  "write docs"  execution=pending_yield  activity=pending_yield
```

The root renders its identity only: it is not a `demi agent` job and has no
supervisor telemetry. `--json` is `{ tree }`: the same nodes as a flat list in
the same order. Each entry names its `subagentId`, `parentSessionId`, kind
(`root`, `live`, or `archived`), description, profile, and phase; an archived
entry carries `closedAgoMs`, and `self` marks the caller. Every age is relative
to the query instant. It is a snapshot, not a wait, and not for polling loops.

### `demi agent show <id>`

The session-content read, valid for any live agent except the root. Every
duration is relative to the query instant, never a wall-clock timestamp. The
caller has no other clock for the target; this snapshot is how it tells motion
from stall.

It returns only:

- the job: ID, parent, description, profile, phase, and time elapsed since
  spawn;
- `execution`: `idle`, `provider_streaming`, `tool_executing`, `compacting`,
  `finalizing`, or `pending_yield`. This is an observation the supervisor
  derives, not the session phase (`idle`, `running`, `compacting`) that `phase`
  frames carry;
- how long the current `execution` state has lasted: this stream, this tool, or
  this yield wait;
- the time since the last child event (a tool start or end, or assistant text);
- the current activity: the in-flight tool's title, or `streaming`, or the
  execution state;
- at most the last 8 tool calls, with their titles and status but no results,
  each with its own duration and how long ago it ended; for the in-flight one,
  how long it has been running. A call's title is the trimmed `description`
  argument it declared, else the tool's name. When older calls are dropped, the
  in-flight one is kept;
- the last assistant text, with the same 32 KiB bound as a completion, and how
  long ago it was produced.

```text
id: ag_h2c5
parent: ag_x7f9k2
description: check the token parser
profile: (inherit)
phase: running
elapsed: 14m
execution: tool_executing (for 12m)
last-event: 12m ago
activity: run the integration suite
recent tool calls (last 2):
  [completed in 3s, ended 12m ago] list call sites of parseToken
  [executing for 12m] run the integration suite
last assistant text (13m ago):
Found 14 call sites; running the integration suite next.
```

A target whose last event was 8 seconds ago is working. A `tool_executing`
state that has lasted 12 minutes on the same title is stuck. Counts without
ages cannot tell those apart.

It does not return tool output bodies, file contents, thinking, or older turns.
A missing or archived ID fails. `--json` is `{ agent }` with the same fields,
durations as millisecond offsets from now.

`show` is for deciding the next action, such as `send` or `abort`. It is not a
completion channel or a polling loop. The supervisor delivers completion
messages independently of shell commands.

The field descriptions of `list` and `show` state that they are snapshots, that
`show` omits tool outputs, and that they are not for polling.

## Profiles

A profile is harness configuration: data the harness declares, not
configuration a command invents.

| Field | Meaning |
| --- | --- |
| Name | The `--profile` value. `default` is reserved: it names no profile, and a harness that declares a profile named `default` fails at assembly. |
| Description | What the profile is for. |
| System prompt | Optional. Replaces the parent's system prompt and drops the parent's preamble; the command help and the subagent preamble are still supplied. |
| Commands | Optional. Narrows the parent's harness commands for the child. |
| Spawning | Whether children of this profile may spawn children of their own. |
| Model | Optional. A model selection used instead of the parent's; the child still runs on a [runtime fork](../providers/providers.md#runtime-forks-and-closing) of the parent's provider runtime. |

The coding harness declares no named profiles. Coding tasks spawn children
without `--profile`, which inherit the parent's coding instructions and its
ability to edit files.

Omitting `--profile` always selects the unnamed inherit profile: the parent's
harness prompt and preamble, model, Host, and commands. It exists whether or
not the harness declares profiles, and it cannot be configured or replaced. A
given `--profile` must match a declared name; an unknown name fails and lists
the available ones. A profile's command narrowing applies to the harness
commands; the `demi agent` group is grafted after it and cannot be narrowed
away. The only sanctioned narrowing of `demi agent` is the spawn restriction
(`--no-subagents`, or a profile that forbids spawning), which removes `spawn`,
`abort`, and `resume`; communication and reads always remain. The restriction
persists with the child across restore and resume.

The node assembly supplies every node's provider runtime. A profile may pin a
model selection; the running command cannot pick a provider. Profiles apply to a
session's own children; a subagent spawning grandchildren resolves names
against the same harness profile list.

## Child identity

A child's jobs carry a
[command context](../execution/native-runtime.md#command-context) whose
`caller` is `agent` with the child's node, the same ID as `subagentId`. There
is no depth marker: depth has no behavioral meaning.

## Child context

A child starts with an empty transcript. Isolation is the point: the parent's
conversation, already-read files, and tool history do not cross. The context
that the child needs is assembled in three layers. Demi does not rewrite or
summarize the parent transcript into the child.

| Layer | Owner | Content |
| --- | --- | --- |
| System prompt | The profile, else the parent harness | Worker identity, shell rules, and the rendered command help. A profile's system prompt replaces the parent's; the command help is still supplied. |
| Preamble | The agent server, for every child | This session is a subagent; its ID and its parent's ID; ending the turn with nothing pending returns the last assistant text as the result; `demi agent send` reaches the parent (`parent`) and any agent in `demi agent list`; spawn delegates further, or this session may not spawn; the session is not talking to the product user and does not address them. |
| First user message | The parent model | The spawn prompt from stdin. Demi does not inspect or pad it. |

The inherit profile carries the parent's system prompt, so the child already
knows the shell session rules and the registered commands. A named profile that
only states a role still uses that inherited prompt unless it sets its own
system prompt.

The agent server does not inject project instruction files, git status, parent
memory, or a roster dump. A harness that loads those for the parent, for
example in its system prompt or preamble, loads them for a child that inherits
that harness. A profile that replaces the system prompt opts out. Explore-style
profiles that want a cheap, instruction-light worker replace the prompt; a
profile is a prompt and a command set, never a restriction the Host enforces.

Never copied into a child:

- the parent transcript and tool results;
- skills or files already in the parent's context;
- the parent's output style or product-user voice;
- a fork of the parent system prompt when the profile replaced it.

## Abort

`demi agent abort <id>` and an explicit abort of the parent's tree stop the
named node and its descendants. Siblings are untouched. An abort, like a
terminal failure, first stops what the child still runs, which records the
stop in its transcript, and then closes the child's own children. Their
completions are saved in the child's final checkpoint as waiting input and
open no turn; the child's next round reads them. A finished creation
command's cancellation has no authority over its child. Dispose saves the
checkpoints and detaches a subtree for restore; it does not archive it as
aborted.

The browser stops children with two client frames. `abort_subagents` aborts
every live child of the root, and `abort_subagent` with a `subagentId` aborts
that child of the root; each takes the child's subtree with it. Neither has a
reply of its own: the `subagent` `closed` frames report the result.

Each closed child emits a `closed` event with the phase `completed`,
`aborted`, or `error`, and a completion message with the outcome `completed`
and the result, `aborted`, or `failed` and the failure text. A successful
creation command keeps exit code zero even if the child later fails.

## Persistence

A session tree is stored as nodes in one store: the tree store the backend
supplies for each conversation, over the conversation's database
([Storage](../backend/storage.md#conversation-state-and-transactions)). A node
row carries identity and relationship: the ID, the parent ID (none for the
root), the description, the profile, the spawn time (the round), the spawn
restriction, and, once closed, its phase, time, bounded result or failure text,
and whether its completion was delivered. Beside it is the node's checkpoint:
its transcript rows, its state row, and its command state
([Tree store](runtime.md#tree-store)). Parent and child are related by a
column, never by a key path. Nothing about a node depends on a Host: a Host
executes, the store remembers, and a node is readable while its target is
offline.

```text
conversation c1's store (the root node's ID is the conversation's)
  node c1  parent=null              root: blocks, state
  node a2  parent=c1  closed=completed  result="…"  delivered=1
  node a3  parent=c1                live: blocks, state (queue holds its brief until the first save)
  node a4  parent=a3                live
```

Three commits are atomic, whatever the store:

| Commit | Rows written together |
| --- | --- |
| **Create** | The node row with its parent link, the initial state, and the first message, queued in the checkpoint, so a child the process loses before its first turn still has its brief. |
| **Save** | The changed block rows, the state row, and the consumption of every completion message the checkpoint now carries (below). |
| **Close** | The closed phase, the time, the bounded result or failure text, and the completion marked not yet delivered. |

A close follows the node's final checkpoint, which its session saves when it is
disposed. A process that ends between the two leaves a live node that is
quiescent, and the next restore closes it with the same result, so the two
orders cannot be told apart.

A closed round remains undelivered until the parent's checkpoint carries its
completion receipt as a pending agent input or an `agent_message` block. The
save marks that exact child round delivered in the same commit. Restore
delivers every still-undelivered completion.

**Restore.** Reopening a root node restores its live children, each of which
restores its own: a tree restore, with one rule per node.

- A turn the process interrupted resumes from its resume point
  ([Recovery is one mechanism](failures-and-recovery.md#recovery-is-one-mechanism)).
- The messages queued in the checkpoint are sent again, in order.
- Saved yield wakeups are armed again, and one already due fires at once, so a
  child waiting on a wakeup keeps waiting instead of closing
  ([Yield wakeups](runtime.md#yield-wakeups)).
- A child that is quiescent closes with its result.
- A live child that cannot be rebuilt, for example because the harness does
  not declare its profile, is deleted with its subtree.

The root's interrupted turn is its client's to resume: the root records the
interruption, and the browser offers Resume
([Recovering an unfinished turn](../product/product.md#recovering-an-unfinished-turn)).
The root's queued messages run, and its pending agent input and due wakeups
wake it only when its last turn was not interrupted.

**Archive.** A closed child is archived: its rows stay, marked with the closed
phase. `demi agent list` shows archived children, and restore skips them.
Nothing prunes the archive: an archived child lives exactly as long as its root
and is deleted only with it, so a revivable ID stays revivable.
`demi agent resume <id>`, run by the archived child's parent with the message on
stdin, revives it in one commit: the node row is live again with a fresh spawn
time, which starts the new round, and the message is queued in the checkpoint.
The session rebuilds from the preserved transcript, and the message opens its
next turn on top of it. The command returns the child ID; the new round
delivers its completion separately.

**Eviction.** A connection that closes only detaches, and a tree that has been
detached and quiescent for 10 minutes is disposed
([Connections and the live tree](runtime.md#connections-and-the-live-tree)). A
tree with a live child is not quiescent, so eviction only ever disposes an idle
root. Disposing it saves its checkpoint, and the next open restores the tree by
the rules above, exactly as after a restart.

## Runtime

Every agent is one kind of node, built by one assembly: the same session, the
same store contract, the same standard tools over per-Host shell environments
with handle ownership checks, the same supervision of its own children, the
same `demi agent` group, and the same events. Parent and child are a
relationship that the tree manages (delegation, messages, and result delivery),
never a second runtime. The supervisor asks the assembly for a node and never
builds one itself; only the node assembly creates a session.

What differs between nodes is configuration: the prompt (a profile's or the
parent's, with the subagent preamble on top), the model (the profile's or the
parent's), the commands (the profile's narrowing of the parent's), the spawn
restriction, and the node's role, which sets its lifecycle policy: a child
resumes its interrupted turn on restore and closes when it is quiescent; the
root leaves both to its client. The role is a node option, not a depth.

- The provider runtime is a runtime fork of the parent's
  ([Runtime forks and closing](../providers/providers.md#runtime-forks-and-closing));
  the transcript starts empty.
- A node inherits the spawner's cwd. Every node reaches its Host through the
  conversation's host access, because the execution target belongs to the
  conversation ([Resolve a target](../execution/sessions-and-targets.md#resolve-a-target)).
- Each node has at most 8 live children. The limit is the same for every node
  in the tree, and no command changes it. `spawn` and `resume` fail when it is
  reached.
- The shell preview budget ([Tools](runtime.md#tools)) applies to every node,
  measured against that node's current model's context window.
- Dispose detaches the subtree without closing it: checkpoints are saved, and
  the next open restores the subtree. Abort closes it.

### Creation command ownership

`spawn` and `resume` are short `rpc` commands. The shell owns only the creation
request and its response. The supervisor owns the persisted child and its
completion delivery. Once the start reservation is committed, creating or
reopening the child runs to completion even if the invoking call is cancelled:
the child survives an abort of the shell job, and a retry with the same request
ID returns it. A start under way counts as a live child of its owner: the owner
does not close under it, and an abort of the owner waits for it and then closes
the new child with the rest of the subtree. File and process work that the child performs uses its own
runner-backed shell environments.

## Protocol

Subagent traffic for the whole tree travels on the conversation's socket,
beside the root's frames ([Frame protocol](runtime.md#frame-protocol)). Each
session's inference transcript is only its own transcript frames.

| Server frame | Fields | Sent |
| --- | --- | --- |
| `subagent` | `event` (`started` or `closed`), `job` | When a child starts (spawn, resume, or restore) and when it closes |
| `subagent_transcript_reset` | `subagentId`, `blocks`, `revision`, optional `failures` | After `started`, and for every live child when the browser opens the conversation or syncs its transcript |
| `subagent_transcript_patch` | `subagentId`, `patches`, `revision`, optional `failures` | When a child's transcript changes |

A job describes one child:

| Field | Meaning |
| --- | --- |
| `subagentId` | The child's node ID |
| `parentSessionId` | The node that spawned it |
| `description` | The spawn's `--description`, or empty |
| `profile` | The profile name, or null for the inherit profile |
| `phase` | `running`, `completed`, `aborted`, or `error` |
| `startedAt` | The round's start time, RFC 3339 |
| `endedAt` | The close time, or null while the child runs |
| `result` | Only on a `completed` close: the child's last assistant text, at most 32 KiB |

Patches and revisions follow the root's rules
([Frame protocol](runtime.md#frame-protocol)), separately for each subagent
stream. `failures` is the backend's reading of the error blocks the frame
carries, attached when the frame is sent and never stored; root transcript
frames carry the same field ([Failure facts](../backend/backend.md#failure-facts)).
A child's other session events, such as phase, queue, errors, and shell output,
send no frames. `AgentClient` in `@demicodes/agent-client` mirrors the three
frames as client events; its transcript events omit `revision`, as the root's
do.

`parentSessionId` is the tree: the browser keys nested UI by it. Frames from any
depth are flat on the connection; the protocol has no per-level nesting. A
child's blocks are the same types as the root's
([Transcript](runtime.md#transcript)). They never appear in another session's
inference transcript.

An incorporated agent message is a transcript patch that adds an
`agent_message` block to the target's stream: the root's own transcript frames,
or `subagent_transcript_patch` for a subagent target. It is not a fourth
subagent event.

When the browser opens the conversation, Demi sends the root's transcript and
state frames and then, for each live agent of the tree in depth-first spawn
order, `subagent` with `started` and `subagent_transcript_reset`. A transcript
sync replays the live children the same way. The browser's two frames for
stopping children are described in [Abort](#abort).

## Sequence

```text
transcript_patch                 parent tool_call shell_exec executing
subagent started                 job.subagentId=ag_1  phase=running
subagent_transcript_reset        subagentId=ag_1  blocks=[]
transcript_patch                 parent creation tool_call completed, stdout=subagentId
subagent_transcript_patch        subagentId=ag_1  + user (the task brief)

subagent_transcript_patch        subagentId=ag_1  + tool_call executing
subagent_transcript_patch        subagentId=ag_1  tool_call completed
subagent_transcript_patch        subagentId=ag_1  + text

transcript_patch                 parent + agent_message (child `send parent`)
subagent_transcript_patch        subagentId=ag_1  + agent_message (parent `send ag_1`)

subagent_transcript_patch        subagentId=ag_1  + text (final)
subagent closed                  phase=completed  result=...
transcript_patch                 parent + agent_message (completion receipt), at its next boundary or through an internal continuation
```

## Product rendering

Agent-originated inputs render as receipts: collapsed agent rows that name the
sender and the event, such as "UI implementation sent an update", "UI
implementation completed", or "UI implementation failed", and expand to show
only the message or the result. Sender IDs, rounds, and timestamps stay in the
data. A receipt never renders as a user bubble and offers no pending-steer,
edit, or Fork control: it is inspectable context, not user input. The gallery
shows every variant.

- A receipt appears once, where it entered context. A child's `closed` event
  updates the dock's agents chip and the children's history; it does not add a
  second receipt.
- The user's message queue stays human-authored. Requesting, thinking, tool
  execution, and receipt rows keep their distinct meanings.
- No receipt content is discarded to reduce visual noise; the collapsed row is
  the quiet default, and the complete message stays inspectable.

Nested tool use renders from the same blocks on `subagent_transcript_*`, keyed
by `subagentId` under the matching `subagent` `started` job, with
`parentSessionId` giving the tree.

Root assistant text is the only user-visible reply stream. A child's `text`
blocks are for nested UI, such as cards and inspection, not a second
user-facing reply.

The dock's agents chip counts live children only and is hidden when none is
live. Completed, aborted, and failed children remain available as history but
do not count. A creation command exits independently and adds nothing to the
Running terminal count for its child's lifetime.

## Layering

| Part | Responsibility |
| --- | --- |
| Command system | Declarations, `rpc` dispatch through the serializable handler interface, and command storage as messages |
| Agent runtime | The node assembly, supervision and the agent directory, the `demi agent` group, agent messages, the subagent frames, and the tree store contract |
| Backend | The tree store over the conversation's database, and the conversation's host access for every node |
| Coding harness | No named profiles; children inherit by default |
| `web-ui` | Nested subagent views, receipts, and the agents chip over `AgentClient` |

[Crates](../architecture/crates-and-packages.md#crates) names the crate that owns
each part.

## Non-goals

- New model-facing tools.
- A runtime `--model` option or provider picker.
- A session copy as spawn.
- Depth caps or per-depth command stripping.
- Resident actor children. Accepted messages defer a child's close until they
  are consumed; they do not turn subagents into daemons that idle waiting for
  mail.
- Synchronous messaging (`send --wait`, request and response). Messages are
  fire-and-forget; ask-and-answer is a close followed by `resume`.
- Addressing across roots: the directory scopes one conversation's tree.
- Lifecycle authority across agents. Messages are talking; stopping and
  reviving stay with the parent.
- `demi agent wait` or `result`: completion messages deliver the outcome.
- Tailing a child transcript into spawn stdout or into another session's
  inference transcript.

## Acceptance

Acceptance uses scripted providers and isolated stores and runner fixtures,
never a real model.

Agent messages:

1. A busy parent receives an update and a completion before its next inference
   request; neither becomes a separate queued user turn.
2. Several messages admitted before one boundary enter through one
   continuation, in order, with distinct IDs and no repeated acknowledgements.
3. Idle delivery wakes the recipient once; a race between finalization and
   admission loses no message; a late receipt does not undo a user's stop.
4. Tool execution continues uninterrupted, and compaction preserves pending
   agent messages without exposing human pending-steer controls.
5. A restart after the child closes, after the parent admits the receipt, or
   after the receipt is materialized produces one receipt per completion round.
   A resumed child has a new receipt ID; an old acknowledgement cannot consume
   the new round.
6. The provider receives each agent message with its source envelope, and a
   restart recovers pending agent messages from the checkpoint.
7. Human sends and steers keep their behavior, queue controls, and
   cancellation. Agent inputs produce `agent_message` blocks and human steers
   produce `steer` blocks; there is no agent-specific scheduler and no
   duplicate receipt record.

The tree and its commands:

1. A spawned child starts with an empty transcript and the subagent preamble,
   and its first user message is the brief. An empty brief fails; an empty last
   assistant text completes with an empty result.
2. A grandchild spawns, is aborted with its parent's subtree, and is restored
   with the tree.
3. `spawn` and `resume` fail at the live-children limit.
4. `send` and `show` reach siblings across branches; `abort` and `resume` of a
   node that is not the caller's child are refused; a send to an archived agent
   is refused; a child with unread messages does not close.
5. An idle parent wakes when a child completes.
6. Subagent frames arrive from every depth, and reopening the conversation
   replays the live tree.
7. An inherited prompt and a replaced prompt reach the child as
   [Child context](#child-context) describes; an unknown profile is refused
   with the available names, and a profile named `default` fails at assembly.
8. The spawn restriction, from `--no-subagents` or a profile, removes `spawn`,
   `abort`, and `resume`, keeps communication and reads, and survives archive,
   reopen, and resume.
9. Descendants inherit the prompt and the model after command narrowing, and
   the shell preview budget reaches every descendant.
10. `resume` continues the preserved transcript. A request ID retried with the
    same arguments returns the same child without a new round; different
    arguments and a superseded round are refused.
11. A spawn whose reservation is committed survives cancellation of its
    invoking call.
12. `list` renders the tree with the caller's marker; `show` stays within its
    bounds.
13. Disposing a parent detaches its live children instead of aborting them.

Persistence:

1. The three commits hold across a restart: a child lost before its first save
   runs its brief; a close whose delivery was not committed is delivered once
   at restore; a quiescent live child closes at restore.
2. The in-memory and the database realizations of the tree store behave alike:
   creation with the queued brief, a save that marks a carried completion
   delivered, close and reopen, deletion with descendants, a later turn that
   saves only its own rows, and a stale completion that cannot mark a newer
   round delivered.
3. A tree detached and quiescent for 10 minutes is disposed; a detached tree
   with a live child or a scheduled wakeup is not. An evicted tree
   reopens with the same transcripts, command state, model selection, and
   archived children as before eviction.

Product:

1. The product and the gallery show collapsed and expanded updates, completed,
   failed, and aborted receipts, long content, equal descriptions with distinct
   sender IDs, and reconnect.
2. The coding harness declares no named profiles, its children inherit by
   default, and the model sees the spawn prompt field's help.
