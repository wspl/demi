# Subagents

| | |
|---|---|
| Date | 2026-09-01 |
| Status | Design |
| Scope | `@demicodes/agent` child sessions, `@demicodes/shell` long-running registered commands, `demi agent` CLI, `AgentClient` subagent events |

A subagent is an isolated child `AgentSession` that any session starts as a
registered command. Sessions form a tree of arbitrary depth rooted at the
product-facing root session: every node of the tree — the root included — is
the same kind of session, built by the same assembly, stored under the same
contract (§ Runtime, § Persistence), carrying the same `demi agent` command
tree and supervising its own children. The model-facing tool surface stays
the five standard tools. Products subscribe to subagent events on the same
`AgentClient` connection as the root session.

This is not `AgentSession.clone()`. Clone copies a conversation prefix for
compaction and recall. A subagent starts with an empty transcript.

## Why

Parent context is expensive. Parallel exploration and focused work need a
fresh session, the same Host, and a result that returns to the parent without
dumping the child's tool history into the parent's inference transcript.

The shell already has start / observe / write / abort / yield. Subagents reuse
that control surface instead of adding `agent_exec` tools.

Depth is not capped. A child delegating a slice of its own task spawns exactly
like the root does; there is no per-depth command stripping. What bounds the
tree is fan-out (`maxLiveSubagents` live children per session) and the real
turns each spawn costs. A spawner can still forbid one specific child from
delegating further — `--no-subagents` at spawn, or `canSpawnSubagents: false`
on the profile — an explicit per-child restriction, never depth-derived: the
child loses spawn, `abort`, and `resume`, and keeps `send` / `steer` /
`list` / `show`.

## Topology and the agent directory

Each root session keeps one **agent directory**: a flat registry
of every live session in the tree, keyed by session id. Each entry carries the
parent session id, description, profile, and phase. Spawn registers, close
unregisters.

The directory is the sole basis for cross-tree addressing. `send`, `steer`,
and `show` resolve their target id against it — any live agent in the tree is
addressable, regardless of the sender's position. There are no routing rules
along the tree.

Authority is split by verb, not by depth:

- **Lifecycle** (`spawn`, `abort`, `resume`) — only on your own direct
  children. Whoever spawns an agent owns its life; nobody else kills or
  revives it.
- **Communication** (`send`, `steer`) and **reads** (`show`, `list`) — any
  live agent in the tree.

## Model-facing surface

The model still sees:

```text
shell_exec
shell_status
shell_write
shell_abort
yield
```

`AgentServer` injects a `demi agent` command into every session registry —
root and subagents receive the identical tree. If the harness already
registered `demi`, the subcommands attach to that tree; otherwise
`AgentServer` registers a `demi` root that only contains them.

```text
demi agent spawn [--profile <name>] [--description <title>] [--no-subagents] [prompt]
demi agent abort <id>
demi agent resume <id> [message]
demi agent send <id|parent> [message]
demi agent steer <id|parent> [message]
demi agent show <id>
demi agent list
```

`demi agent spawn` with a prompt (or stdin) starts a child and waits until that
child session ends. Stdout is the child's last assistant text. Stderr's first
line is `subagentId: <id>`.

While that command is the shell foreground job:

- `shell_write` steers the child (each chunk is one chime-in)
- `shell_abort` aborts the child (and its subtree)
- `shell_status` is process liveness (`running` / `exited`). Stdout stays
  empty until the command exits.

Parallel children are multiple `demi agent` invocations in separate
`shell_exec` calls; a busy shell session gets a fresh one automatically.
`timeoutMs` on spawn is only an observation window, capped at
`MAX_TIMEOUT_MS` per call like every exec wait: the call returns a running
status at the cap and the parent turn continues; the child keeps running.
A short window is the idiomatic way to fan out.

Spawn is an `rpc` command: on a machine the command-mode process relays it
to the backend and stays attached for its stdin (`runner.md` § The local
relay); hostless, the backend calls it directly. `demi agent … &` is not a
spawn path: a backgrounded job's stdin is not the tool call's, so nothing
steers the child. The short subcommands below are fine either way.

Prompt and `send` / `steer` / `resume` messages use an optional positional, or
stdin/heredoc when the positional is omitted. An empty message fails.
`--profile` names a profile configured at harness assembly. `--no-subagents`
forbids the child from spawning children of its own. There is no `--model`
flag: model and provider runtime come from the profile or from the parent.

`--json` on spawn and `resume` writes `{ "subagentId", "text" }` at exit.
`abort` / `send` / `steer` / `show` / `list` define JSON objects
`{ id, aborted }`, `{ id, accepted }`, `{ id, accepted }`, `{ agent }`, and
`{ tree }` respectively.

### Command help

How to write the child's task brief lives only on the spawn `prompt` parameter
(`positional`, `stdinField`). That text is the child's first user message.
Help stays declarative: no invocation examples. Demi does not reject a short
prompt.

The field description uses the same four-beat as
`COMPACTION_SUMMARY_INSTRUCTION` (job, stance, inventory, output). Compaction
has the history and must not obey it. A child has no history, so the parent
must put the continuation facts into this argument:

> The child's first user message and only task brief. The child starts with an
> empty transcript and cannot see this conversation: do not refer to prior
> turns, and do not paste this conversation or the product user's message
> unchanged. Include the goal for this child, applicable decisions and
> constraints, whether to edit or only report, how to verify, and every
> concrete identifier it needs (paths, ids, error text, commands already tried
> and their key results). State the exact shape of the last assistant text it
> should return.

`--description` is a short UI title for concurrent children. `--profile` names
a harness profile; the live describe text lists configured names.

## Communication

Two message verbs with distinct delivery moments. Both are fire-and-forget:
they queue and return, never wait on the target — which is also why the mesh
cannot deadlock. Both deliver an ordinary **user** message into the target
transcript, prefixed `[agent <id> — <description>]` so concurrent
correspondents are distinguishable. `parent` is an alias resolving to the
sender's spawning session.

### `demi agent steer <id|parent>` — chime in

Injects the message into the target's **currently running turn**: the target
sees it at the next sampling/tool boundary (or live, on a provider steer
stream) and continues its current work with the new information. Nothing is
cancelled and the turn does not restart — steering is talking to someone while
they work, not stopping them. Use it to course-correct, add a constraint, or
redirect effort mid-flight.

The target must be inside a turn. Steering an idle root session fails — there
is no turn to join, and silently downgrading to a mailbox drop would falsify
the "seen now" intent. The error says to use `send`.

### `demi agent send <id|parent>` — leave a message

Queues the message into the target's **inbox**; it is seen as a fresh user
turn at the target's next turn boundary, never mid-turn. Use it for progress
reports, handing over results, and non-urgent questions.

- Root session: idle → the message wakes it as a new turn; busy → it waits in
  the inbox until the current turn ends.
- Subagent: a non-empty inbox **defers closing**. Session end requires
  quiescence (idle, no pending yields) *and* an empty inbox; when a turn ends
  with mail waiting, the supervisor opens a new user turn with the queued
  messages instead of closing the session. A message therefore extends the
  child's life by one turn — without anyone sending, child lifecycle is
  unchanged. This also closes the delivery race: a send that lands while the
  target is finishing either makes it into the inbox (and is answered) or
  fails loudly because the target is already archived. Nothing is dropped
  silently.
- Archived target → error, pointing at that agent's parent as the only party
  who can `demi agent resume` it.

### Result

There is no result command. The child's natural session end (quiescent, empty
inbox) is the result: `demi agent spawn` exits 0 and stdout is the child's last
assistant text, truncated at 32 KiB. Empty last text is a valid 0 exit. A
child `yield` keeps the spawn command running; the child is not finished.

If the parent is still blocked in that `demi agent spawn` invocation, the tool
result is the return path. If the parent is idle, Demi also delivers a user
send so the parent is woken; the body carries `subagentId`, description, and
the same result text. The store records which path took the completion, so a
process ending between the child's close and the parent's next checkpoint
loses nothing (§ Persistence).

`AgentServerOptions.subagents.notifyParentOnIdle: false` leaves root-session
wakeups to the product. Parents inside the tree always wake on child
completion so they can integrate the result and finish their own work.

Ask-and-answer needs no live channel: a child that needs a decision ends its
turn with the question as its last assistant text — the session closes, the
question returns as the result, and the parent answers with
`demi agent resume <id> <answer>` on top of the preserved transcript.

A parent blocked in a spawn wait does not see incoming sends or steers until
that wait returns. The dead window is bounded: every exec observation window
caps at `MAX_TIMEOUT_MS`, so the wait returns a running status by then and
the parent can take messages again. Background fan-out (short `timeoutMs`,
then end the turn) has no dead window at all.

## Observe

The parent model does not receive `subagent_transcript_*`. Those frames are
for products. Pushing child tool history into the parent inference transcript
is the failure mode this design exists to avoid.

A stuck child also cannot be asked to report on itself: a steer queues until
the child reaches a boundary, so a hung tool or provider stream never
answers. Observation is a supervisor read. It does not inject into the target
transcript and does not wait on it.

Three pulls, none of them a wait:

| | Answers | Does not answer |
|---|---|---|
| `shell_status` on the spawn job | process liveness (`running` / `exited`) | session content |
| `demi agent list` | the whole live tree, plus archived children | one agent's recent work |
| `demi agent show <id>` | bounded snapshot of one live agent | full transcript, tool outputs, thinking |

### `demi agent list`

Renders the session tree of this root session from the root down, marking the
caller's own position. Live agents show phase, execution, elapsed and
last-event ages; each node's archived children render greyed beneath it with
their closed phase and age:

```text
● root
├─● ag_x7f9k2  "refactor auth"  running  up 4m  last-event 8s ago  streaming
│ ├─● ag_m3n8  "search call sites"  running  up 1m  tool_executing ← you
│ └─○ ag_q5w2  archived (completed 3m ago)  "update tests"
└─● ag_z4r6t0  "write docs"  running  up 2m  pending_yield
```

The root node renders identity only — it is not a `demi agent` job and has no
supervisor telemetry. `--json` is `{ tree }`: the same nodes with
`parentSessionId` links and millisecond ages. Every age is relative to the
query instant. A snapshot, not a wait — not for polling loops.

### `demi agent show <id>`

The session-content path, valid for any live agent except the root. Every
duration is relative to the query instant, never a wall-clock timestamp. The
caller has no other clock for the target; this snapshot is how it tells
motion from stall.

It returns only:

- job: id, description, profile, phase, elapsed since spawn
- `execution`: `idle` | `provider_streaming` | `tool_executing` |
  `compacting` | `finalizing` | `pending_yield`. A supervisor-derived
  observation enum, not the core `SessionPhase` (`idle` / `running` /
  `compacting`) already carried by `phase` frames.
- how long the current `execution` state has lasted (this stream, this tool,
  this yield wait)
- time since the last child event (tool start/end or assistant text)
- current activity (the in-flight tool title, or streaming / idle)
- at most the last 8 `tool_call` titles and their status, no results; each
  with its own duration and how long ago it ended (the in-flight one: how
  long it has been running)
- last assistant text, same 32 KiB bound as spawn stdout / `job.result`, and
  how long ago it was produced

A target whose last event was 8s ago is working. A `tool_executing` state
that has lasted 12m on the same title is stuck. Counts without ages cannot
tell those apart.

It does not return tool output bodies, file contents, thinking, or older
turns. A missing or archived id fails. `--json` is `{ agent }` with those
fields as millisecond offsets from now.

`show` is for deciding the next verb (send, steer, abort, yield, or ignore) —
look before you talk. It is not a completion channel and not a loop. Waiting
is still blocking spawn, `shell_status` with `timeoutMs`, or `yield`.
Completion and abort still arrive as the spawn tool result, or as a user send
when the parent is idle.

The shell result's generic running hint ("check again with shell_status, or
call yield") is exactly the polling loop this table exists to prevent, so the
spawn and `resume` commands set `Command.runningHint` — a per-command override
of that line, surfaced on running `ShellCommandStatus` while the command is
the foreground job — telling the parent to steer, abort, or end the turn and
be woken, never to poll.

`list` / `show` field descriptions state they are snapshots, that `show`
omits tool outputs, and that they are not for polling.

## Profiles

Profiles are harness configuration, not CLI-invented configuration.

```ts
interface SubagentProfile {
  name: string
  /** Shown in `demi agent --help` so the parent model can choose. */
  description: string
  systemPrompt?: AgentHarness<State>['systemPrompt']
  commands?(parent: Command[]): Command[]
  /** When false, children of this profile cannot spawn subagents (communication and reads remain). */
  canSpawnSubagents?: boolean
  /** Same provider runtime as the parent (`provider.clone()`), optional model override. */
  model?: ModelSelection
}

interface AgentHarness<State> {
  // ...
  agents?(ctx: AgentHarnessContext<State>): SubagentProfile[] | Promise<SubagentProfile[]>
}
```

Omitting `--profile` always selects the unnamed inherit profile: the parent
harness, model, Host, and commands. It exists whether or not `agents()` is
declared and cannot be configured or replaced; `default` is a reserved word,
not a profile name, and a harness declaring a profile called `default` fails
at assembly. A given `--profile` must match a declared name. A profile's `commands()` filter applies to the
harness commands; the `demi agent` tree is injected after it and is not
strippable by `commands()`. The only sanctioned narrowing is the spawn
restriction (`--no-subagents` / `canSpawnSubagents: false`), which removes
spawn, `abort`, and `resume` — communication and reads always remain. The
restriction persists across restore and resume with the child.

The assembler owns provider instances. A profile may pin a `ModelSelection` at
init. The running command cannot pick a provider. Profiles apply to a
session's own children; a subagent spawning grandchildren resolves names
against the same harness profile list.

## Child identity

Each child receives:

| env | value |
|---|---|
| `DEMI_SESSION_ID` | child session id (same as `subagentId`) |
| `DEMI_SUBAGENT_ID` | same id |
| `DEMI_PARENT_SESSION_ID` | spawning session id |

There is no depth marker: depth has no behavioral meaning.

## Child context

A child starts with an empty transcript. Isolation is the point: the parent's
conversation, already-read files, and tool history do not cross. Context that
the child needs is assembled in three layers. Demi does not rewrite or
summarize the parent transcript into the child.

| Layer | Owner | Content |
|---|---|---|
| `systemPrompt` | profile, else parent harness | Worker identity, shell rules, `commandsPrompt`. A custom `profile.systemPrompt` replaces the parent prompt; `commandsPrompt` is still supplied through `AgentSystemPromptContext`. |
| preamble | `AgentServer`, every child | This session is a subagent; its id and its parent's id; ending the turn with an empty inbox returns the last assistant text as the result; `demi agent send` / `steer` reach the parent (`parent`) and any agent in `demi agent list`; spawn delegates further; do not address the product user as the root session. |
| first user message | parent model | The spawn prompt (positional or stdin). Demi does not inspect or pad it. |

The inherit profile carries the parent `systemPrompt` so the child already
knows shell session rules and registered commands. A named profile
that only states a role still uses that inherited prompt unless it sets
`systemPrompt`.

`AgentServer` does not inject project instruction files, git status, parent
memory, or a roster dump. A harness that loads those for the parent (for
example in `systemPrompt` / `preamble`) loads them for a child that inherits
that harness. A profile that replaces `systemPrompt` opts out. Explore-style
profiles that want a cheap, instruction-light worker replace the prompt; a
profile is a prompt and a command set, never a restriction the Host
enforces.

Never copied into a non-clone child:

- parent transcript and tool results
- skills or files already in the parent context
- parent output-style / product-user voice
- a fork of the parent system prompt when the profile replaced it

`AgentSession.clone()` remains the primitive that copies a conversation prefix.
That is compaction and recall, not spawn.

## Abort

Abort is recursive. `demi agent abort <id>`, `shell_abort` on that job, and
parent `abort` / `dispose` stop the named node and every descendant, deepest
first. Siblings are untouched. Each closed node emits `subagent closed`.

Abort is a non-zero spawn exit plus `phase: 'aborted'`. Provider or runtime
failure is `phase: 'error'`, non-zero exit, and the reason on stderr. If the
parent is idle, a user send reports abort or error the same way.

## Persistence

A session tree is stored as **nodes** in one store: the `AgentTreeStore` the
product injects into `AgentServer`, one per root session id (the backend:
the conversation's database, `storage.md`). A node row carries identity and
relationship — id, `parentId` (null for the root), description, profile, the
spawning round's metadata, spawn time, the spawn restriction, and once
closed its phase, time and bounded result — and the node's checkpoint beside
it: the transcript as one row per block and the state row (phase, queue,
cwd, model, harness). Parent–child is a column, never a key path. Nothing
about a node depends on a Host: a Host executes, the store remembers, and a
node is readable while its target is offline.

```text
conversation c1's store
  node a1  parent=null              root: blocks, state
  node a2  parent=a1  closed=completed  result="…"  delivered=1
  node a3  parent=a1                live: blocks, state (queue holds its brief until the first save)
  node a4  parent=a3                live
```

Three commits are atomic, whatever the store:

| Commit | Rows written together |
|---|---|
| **create** | the node row with its parent link, the initial state, and the first message — queued in the checkpoint, so a child the process loses before its first turn still has its brief |
| **save** | the changed block rows, the state row, and the consumption of every completion message the checkpoint now carries (below) |
| **close** | the closed phase, the time, the bounded result, and the completion marked not yet delivered |

A close follows the node's final checkpoint, which its session flushes on
dispose. A process ending between the two leaves a live node that is
quiescent — idle, empty inbox, no live children — and the next restore
closes it with the same result, so the two orders cannot be told apart.

**Completion delivery.** A closed child's result reaches its parent on one
of three paths, and the store records that it has: the parent was busy —
blocked in the spawn, or elsewhere in a turn — and the spawn command's exit
carries the result; the parent was idle and receives a user message whose id
names the child, and the parent's save that carries that message, queued or
as its user turn, marks the completion delivered in the same commit; or the
product took it from the `closed` frame (`notifyParentOnIdle: false`, root
level only). A restore delivers every completion still marked undelivered as
the idle wakeup message. A completion is therefore never lost between the
child's close and the parent's next checkpoint, and never delivered twice.

**Restore.** Reopening a root node restores its live children, each of
which restores its own — a tree restore, one rule per node: a turn the
process interrupted resumes from its resume point (`session/recovery.ts`);
the messages queued in the checkpoint are re-enqueued in order; a child that
is quiescent closes with its result. Live nodes come back with the metadata
of the round that spawned them. The root's interrupted turn is its client's
to resume (`resume`); its queued messages run.

A closed child is **archived**: its rows stay, marked with the closed phase.
Archived children are listed by `demi agent list` and skipped by restore.
Nothing prunes the archive: an archived child lives exactly as long as its
root and is deleted only with it — a revivable id stays revivable.
`demi agent resume <id> <message>` — the archived child's parent only —
revives one in one commit: the node row is live again with this round's
metadata and a fresh spawn time, and the message is queued in the
checkpoint; the session rebuilds from the preserved transcript and the
message opens its next turn on top of it. From there the command behaves
exactly like spawn (foreground job, steers via `shell_write`, result at
exit).

## Runtime

Every agent is one kind of node, built by one assembly
(`packages/agent/src/node/`): the same `AgentSession`, the same store
contract, the same standard tools over per-Host shell environments with
handle ownership checks, the same supervisor for its own children, the same
`demi agent` command tree, the same events. Parent–child is a relationship
the supervisor manages — delegation, messages, result delivery — never a
second runtime; the supervisor asks the assembly for a node and never builds
one itself. What differs between nodes is configuration: the prompt (a
profile's or the parent's, the subagent preamble on top), the model
(`profile.model` or the parent's), the commands (`profile.commands` over the
parent's), the spawn restriction, and the **lifecycle policy** — a child
resumes its interrupted turn on restore and closes when quiescent; the root
leaves both to its client. The policy is a node option, not a depth.

- the provider runtime is `provider.clone()` of the parent's; the transcript
  starts empty
- a node inherits the spawner's cwd and the current action's `metadata`
  (Host routing); every node resolves its Host as the root, since the
  execution target is the conversation's
- at most `maxLiveSubagents` running children per node — an `AgentServer`
  assembly option (default 8), one value for every supervisor in the tree,
  not a CLI flag; spawn and `resume` fail when full
- `tools.shellPreviewBudgetTokens` applies to every descendant using that
  node's current model context window
- dispose detaches the subtree without closing it (checkpoints flush; the
  next open restores); abort closes it

### Registered commands as foreground jobs

`demi agent spawn` is a long-running RPC command. Registered commands use
the same control surface as host processes:

- `CommandRunContext.signal` is the job abort signal
- `CommandIO` writes into the live stdout/stderr accumulator
- stdin after start is a stream; each `shell_write` chunk is one child steer
- `timeoutMs` / `shell_status` / `shell_write` / `shell_abort` apply as they
  do to a host process

## Protocol

No new `ClientFrame`. Subagent traffic for the whole tree is on the root
`AgentClient` connection. Each session's inference transcript is only its own
existing `transcript_*` frames.

```ts
export type SubagentJob = {
  subagentId: string
  parentSessionId: string
  description: string
  profile: string | null
  phase: 'running' | 'completed' | 'aborted' | 'error'
  /** Present on `closed`: last assistant text, at most 32 KiB. */
  result?: string
}

export type ServerFrame =
  | /* existing frames */
  | { type: 'subagent'; event: 'started' | 'closed'; job: SubagentJob }
  | { type: 'subagent_transcript_reset'; subagentId: string; blocks: Block[]; revision: number }
  | { type: 'subagent_transcript_patch'; subagentId: string; patches: TranscriptPatch[]; revision: number }
```

`ClientSessionEvent` mirrors those three (client-side transcript events omit
`revision`, matching `transcript_reset` / `transcript_patch`).

`parentSessionId` is the tree: products key nested UI by it. Frames from any
depth are flat on the connection; there is no per-level nesting in the
protocol. Child `Block` values are the same types as the parent (`tool_call`,
`text`, `error`, …). They never appear in another session's inference
`transcript_*`.

A delivered `send` or `steer` is an ordinary `transcript_patch` (user or
steer block) on the **target's** stream — the root's own `transcript_*`, or
`subagent_transcript_patch` for a subagent target. Not a fourth subagent
event.

On reconnect, Demi sends the root `transcript_reset`, then for each
still-live agent in the tree `subagent started` and
`subagent_transcript_reset`.

### Bounded view

The spawning session's `shell_exec` `tool_call.view` may carry a bounded
`{ kind: 'subagent', subagentId, description, phase, activity }` for collapsed
UI. Live tool history is the `subagent_transcript_*` stream, not an unbounded
`view`. View updates are `replace_block` patches on that `tool_call` (`view`
is not replayed to the model).

## Sequence

```text
transcript_patch                 parent tool_call shell_exec executing
subagent started                 job.subagentId=ag_1  phase=running
subagent_transcript_reset        subagentId=ag_1  blocks=[]

subagent_transcript_patch        subagentId=ag_1  + tool_call executing
subagent_transcript_patch        subagentId=ag_1  tool_call completed
subagent_transcript_patch        subagentId=ag_1  + text

transcript_patch                 parent user turn     (child `send parent`)
subagent_transcript_patch        subagentId=ag_1  + user steer (parent `steer ag_1`)

subagent_transcript_patch        subagentId=ag_1  + text (final)
subagent closed                  phase=completed  result=...
transcript_patch                 parent tool_call completed, stdout=result
```

## Product rendering

Products already render root `tool_call` blocks. Nested tool use is the same
blocks on `subagent_transcript_*`, keyed by `subagentId` under the matching
`subagent started` job, with `parentSessionId` giving the tree.

Root assistant text is the only user-visible product reply stream. Child `text`
blocks are for nested UI (cards, inspect), not a second user-facing reply.

## Layering

| Package | Role |
|---|---|
| `@demicodes/shell` | Foreground registered commands (signal, live IO, stdin stream) |
| `@demicodes/agent` | The node assembly, supervisors, agent directory, `demi agent` injection, protocol frames, the `AgentTreeStore` contract |
| `@demicodes/backend` | The `AgentTreeStore` over `conversations/<id>.sqlite` |
| `@demicodes/coding-agent` | Optional named profile (`explore`) |
| harness / product | Extra profiles, Host wrapping, UI over `AgentClient` |

Only the node assembly instantiates `AgentSession`.

## Non-goals

- New model-facing tools
- Runtime `--model` / provider picker
- `clone()` as spawn
- Depth caps or per-depth command stripping
- Resident actor children: the inbox defers closing by one turn per message,
  it does not turn subagents into daemons that idle waiting for mail
- Synchronous messaging (`send --wait`, request/response): fire-and-forget
  only; ask-and-answer is close + `resume`
- Cross-root addressing: the directory scopes one root session
- Cross-agent lifecycle authority (steering is talking; killing and reviving
  stay with the parent)
- `demi agent wait` / `result` (blocking spawn, `shell_status` / `yield`, and
  natural session end already cover these)
- Tailing a child transcript into spawn stdout or another session's inference
  transcript

## Coverage

- `packages/agent/src/__tests__/subagent.test.ts` — spawn isolation with the child
  preamble on an empty transcript, nested spawn (grandchild) with recursive
  abort and restore, the per-session live-children ceiling, `send` inbox
  semantics (busy queue, idle wakeup, close deferral, archived-target error),
  `steer` mid-turn injection and idle-root rejection, cross-branch `send` /
  `show` between siblings, lifecycle-authority rejection (`abort` / `resume`
  on a non-child), idle parent wakeup on completion, empty prompt fails,
  empty last text exits 0, `subagent*` protocol frames from nested depths,
  inherited vs replaced `systemPrompt` with unknown-profile rejection,
  spawn restriction (`--no-subagents` and profile `canSpawnSubagents: false`)
  with communication intact across archive/reopen/resume, descendant prompt/model
  inheritance after command filtering, recursive shell preview budget propagation,
  `resume` on the preserved transcript, list tree rendering
  with self marker, parent close detaches (not aborts) live children, the
  three commits across a restart: a child lost before its first save runs
  its brief, a close whose wakeup was not committed is delivered once at
  restore, a quiescent live child closes at restore
- `packages/agent/src/__tests__/tree-store.test.ts` — the store contract
  over the in-memory realization: create with the queued brief, save marking
  a carried completion delivered, close and reopen, delete with descendants
- `packages/backend/src/__tests__/storage.test.ts` — the SQLite realization:
  the same contract over the conversation database, media by reference, the
  cold transcript read of the root node
- `packages/host-virtual/src/__tests__/hostless-environment.test.ts` — registered command abort
  signal, live stdout, `shell_write` as stdin stream, byte-clean pipes around
  a virtual foreground job
- `packages/coding-agent/src/__tests__/coding-harness.test.ts` — unnamed inherit / `explore`
  profiles and the injected `demi agent` prompt-field help
