# Subagents

| | |
|---|---|
| Date | 2026-08-26 |
| Status | Design |
| Scope | `@demicodes/agent` child sessions, `@demicodes/shell` long-running registered commands, `demi agent` CLI, `AgentClient` subagent events |

A subagent is an isolated child `AgentSession` that the parent starts as a
registered command. The model-facing tool surface stays the five standard
tools. Products subscribe to subagent events on the same `AgentClient`
connection as the parent session.

This is not `AgentSession.clone()`. Clone copies a conversation prefix for
compaction and recall. A subagent starts with an empty transcript.

## Why

Parent context is expensive. Parallel exploration and focused work need a
fresh session, the same Host, and a result that returns to the parent without
dumping the child's tool history into the parent's inference transcript.

The shell already has start / observe / write / abort / yield. Subagents reuse
that control surface instead of adding `agent_exec` tools.

## Model-facing surface

The model still sees:

```text
shell_exec
shell_status
shell_write
shell_abort
yield
```

`AgentServer` injects a `demi agent` command into the session registry. If the
harness already registered `demi`, the subagent subcommands attach to that
tree. Otherwise `AgentServer` registers a `demi` root that only contains them.

`demi agent` with a prompt (or stdin) starts a child and waits until that child
session becomes idle after its opening turn, with no pending yield. Stdout is
the child's last assistant text. Stderr's first line is `subagentId: <id>`.

While that command is the shell foreground job:

- `shell_write` steers the child
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

Addressing by id (a later `shell_exec`, or a command-mode process on a host
subprocess):

```text
demi agent [--profile <name>] [--description <title>] [prompt]
demi agent steer <id> [message]
demi agent abort <id>
demi agent list
demi agent show <id>
demi agent send-parent [message]
```

`send-parent` is the only `demi agent` subcommand on a child session. Spawn,
`steer`, `abort`, `list`, and `show` are root-only. Enforcement is the
supervisor's, not the registry's: spawn resolves the session that owns the
invoking shell and rejects any session at depth ≥ 1. Registry visibility
(child registry has no spawn) and the per-child bridge bin dir
(`bridge-bin/<childSessionId>/`, child shells never receive the parent's
bridge env) are ergonomics on top, not the boundary.

Prompt and `send-parent` / `steer` messages use an optional positional, or
stdin/heredoc when the positional is omitted. An empty spawn prompt fails.
`--profile` names a profile configured at harness assembly. There is no
`--model` flag: model and provider runtime come from the profile or from the
parent.

`--json` on spawn writes `{ "subagentId", "text" }` at exit. `steer` / `abort` /
`list` / `show` / `send-parent` define JSON objects `{ id, accepted }`,
`{ id, aborted }`, `{ agents }`, `{ agent }`, `{ accepted }` respectively.

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

## Observe

The parent model does not receive `subagent_transcript_*`. Those frames are
for products. Pushing child tool history into the parent inference transcript
is the failure mode this design exists to avoid.

The child also cannot be asked to report on itself: `steer` queues until the
child can take a turn, so a stuck tool or a hung provider stream never
answers. Observation is a supervisor read. It does not inject into the child
transcript and does not wait on the child.

Three pulls, none of them a wait:

| | Answers | Does not answer |
|---|---|---|
| `shell_status` on the spawn job | process liveness (`running` / `exited`) | session content |
| `demi agent list` | roster of this parent's running children | one child's recent work |
| `demi agent show <id>` | bounded snapshot of one running child | full transcript, tool outputs, thinking |

`demi agent` writes `subagentId: <id>` to stderr when the child starts. While
the child is running, stdout is empty. The last assistant text is written to
stdout only when the spawn command exits. Live activity is not tailed into
stdout: a delta log trains the parent to poll and accumulate the child's work.

`list` prints one line per running child: id, job phase, elapsed since spawn,
time since last child event, profile, description, execution, current
activity. `--json` is `{ agents }`. Finished children are absent (no
checkpoint).

`show` is the session-content path. Every duration is relative to the query
instant, never a wall-clock timestamp. The parent model has no other clock
for the child; this snapshot is how it tells motion from stall.

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
- at most the last 8 child `tool_call` titles and their status, no results;
  each with its own duration and how long ago it ended (the in-flight one:
  how long it has been running)
- last assistant text, same 32 KiB bound as spawn stdout / `job.result`, and
  how long ago it was produced

A child whose last event was 8s ago is working. A `tool_executing` state
that has lasted 12m on the same title is stuck. Counts without ages cannot
tell those apart.

It does not return tool output bodies, file contents, thinking, or older
turns. A missing or finished id fails; there is no stored snapshot after
close. `--json` is `{ agent }` with those fields as millisecond offsets from
now.

`show` is for deciding the next verb (steer, abort, yield, or ignore). It is
not a completion channel and not a loop. Waiting is still blocking spawn,
`shell_status` with `timeoutMs`, or `yield`. Completion and abort still
arrive as the spawn tool result, or as a user `send` when the parent is idle.

`list` / `show` field descriptions state they are snapshots of running
children, that `show` omits tool outputs, and that they are not for polling.

## Profiles

Profiles are harness configuration, not CLI-invented configuration.

```ts
interface SubagentProfile {
  name: string
  /** Shown in `demi agent --help` so the parent model can choose. */
  description: string
  systemPrompt?: AgentHarness<State>['systemPrompt']
  commands?(parent: Command[]): Command[]
  /** When true, `host()` must reject writes. Command allowlists are not enough. */
  readonly?: boolean
  /** Same provider runtime as the parent (`provider.clone()`), optional model override. */
  model?: ModelSelection
}

interface AgentHarness<State> {
  // ...
  agents?(ctx: AgentHarnessContext<State>): SubagentProfile[] | Promise<SubagentProfile[]>
}
```

Omitted `agents()` yields one implicit profile named `default`: inherit the
parent harness, model, Host, and commands, then strip spawn and attach
`send-parent`. `--profile` is optional and must match a configured name.

The assembler owns provider instances. A profile may pin a `ModelSelection` at
init. The running command cannot pick a provider.

## Child identity

Each child receives:

| env | value |
|---|---|
| `DEMI_SESSION_ID` | child session id (same as `subagentId`) |
| `DEMI_SUBAGENT_ID` | same id |
| `DEMI_PARENT_SESSION_ID` | parent session id |
| `DEMI_SUBAGENT_DEPTH` | `1` for a child of the root session |

## Child context

A child starts with an empty transcript. Isolation is the point: the parent's
conversation, already-read files, and tool history do not cross. Context that
the child needs is assembled in three layers. Demi does not rewrite or
summarize the parent transcript into the child.

| Layer | Owner | Content |
|---|---|---|
| `systemPrompt` | profile, else parent harness | Worker identity, shell rules, `commandsPrompt`. A custom `profile.systemPrompt` replaces the parent prompt; `commandsPrompt` is still supplied through `AgentSystemPromptContext`. |
| preamble | `AgentServer`, every child | This session is a subagent; id; `send-parent` talks to the parent when the parent is not blocked in this spawn; ending the turn returns the last assistant text as the result; do not address the product user as the root session. |
| first user message | parent model | The spawn prompt (positional or stdin). Demi does not inspect or pad it. |

The implicit `default` profile inherits the parent `systemPrompt` so the child
already knows shell session rules and registered commands. A named profile
that only states a role still uses that inherited prompt unless it sets
`systemPrompt`.

`AgentServer` does not inject project instruction files, git status, parent
memory, or a sibling roster. A harness that loads those for the parent (for
example in `systemPrompt` / `preamble`) loads them for a child that inherits
that harness. A profile that replaces `systemPrompt` opts out. Explore-style
profiles that want a cheap, instruction-light worker replace the prompt and
keep the Host read-only.

Never copied into a non-clone child:

- parent transcript and tool results
- skills or files already in the parent context
- parent output-style / product-user voice
- a fork of the parent system prompt when the profile replaced it

`AgentSession.clone()` remains the primitive that copies a conversation prefix.
That is compaction and recall, not spawn.

## Parent communication

### `demi agent send-parent`

Delivers a **user** message into the parent session (not a hidden yield wakeup):

- parent turn running → `steer`
- parent idle → `send` (new user turn)

The text includes `subagentId` and description so concurrent children are
distinguishable. Steer that arrives while the parent is inside a tool call
queues and materializes after that tool, using existing steer rules.

A parent still blocked in this child's `demi agent` therefore does not see
`send-parent` until that wait returns. The dead window is bounded: every exec
observation window caps at `MAX_TIMEOUT_MS`, so the wait returns a running
status by then and the parent can take steers again. `send-parent` is for a
parent that continued (short `timeoutMs`, a lapsed window, another turn, or
idle). A child whose parent is waiting on this spawn returns via last
assistant text, not via `send-parent`.

### Result

There is no result command. The child's natural session end (opening send
completed, idle, no pending yield) is the result: `demi agent` exits 0 and
stdout is the child's last assistant text, truncated at 32 KiB. Empty last
text is a valid 0 exit. A child `yield` keeps the parent command running; the
child is not finished.

If the parent is still blocked in that `demi agent` invocation, the tool result
is the return path. If the parent is idle, Demi also delivers a user `send` so
the parent is woken; the body carries `subagentId`, description, and the same
result text.

Abort is a non-zero exit plus `phase: 'aborted'`. Provider or runtime failure
is `phase: 'error'`, non-zero exit, and the reason on stderr. If the parent is
idle, a user `send` reports abort or error the same way.

## Abort

Abort is recursive. `demi agent abort <id>`, `shell_abort` on that job, and
parent `abort` / `dispose` stop the named node and every descendant. Siblings
are untouched. Each closed node emits `subagent closed`.

## Runtime

`AgentServer` is the only place that instantiates `AgentSession`. It owns a
per-parent supervisor:

- `provider.clone()`, empty transcript, no `store` (not listed by
  `listConversations`, not independently resumed)
- inherit parent cwd and the current action's `metadata` (Host routing)
- depth default 1 (child registry has no spawn, steer, abort, list, or show)
- at most 8 running children per parent session; spawn fails when full
- parent dispose aborts the tree
- children are in-memory. Process restart does not revive them. Restoring a
  parent checkpoint closes any still-executing spawn tool_call as aborted.

`AgentSession.clone()` stays the compaction / recall primitive.

### Registered commands as foreground jobs

`demi agent` is a long-running in-process command. Today only `hostSpawn`
creates a `ForegroundProcess`. Subagents require registered commands to use
the same control surface:

- `CommandRunContext.signal` is the job abort signal
- `CommandIO` writes into the live stdout/stderr accumulator
- stdin after start is a stream; each `shell_write` chunk is one child steer
- `timeoutMs` / `shell_status` / `shell_write` / `shell_abort` apply as they do
  to a host process

## Protocol

No new `ClientFrame`. Subagent traffic is on the **parent** `AgentClient`.
The parent's inference transcript is only the existing `transcript_*` frames.

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

Child `Block` values are the same types as the parent (`tool_call`, `text`,
`error`, …). They never appear in the parent's inference `transcript_*`.

`send-parent` is an ordinary parent `transcript_patch` (user `steer` or
`user` block), not a fourth subagent event.

On parent reconnect, Demi sends the parent `transcript_reset`, then for each
**still-running** child `subagent started` and `subagent_transcript_reset`.
Dead children are not reconstructed; they have no checkpoint.

### Bounded view

The parent `shell_exec` `tool_call.view` may carry a bounded
`{ kind: 'subagent', subagentId, description, phase, activity }` for collapsed
UI. Live tool history is the `subagent_transcript_*` stream, not an unbounded
`view`. View updates are `replace_block` patches on that parent `tool_call`
(`view` is not replayed to the parent model).

## Sequence

```text
transcript_patch                 parent tool_call shell_exec executing
subagent started                 job.subagentId=ag_1  phase=running
subagent_transcript_reset        subagentId=ag_1  blocks=[]

subagent_transcript_patch        subagentId=ag_1  + tool_call executing
subagent_transcript_patch        subagentId=ag_1  tool_call completed
subagent_transcript_patch        subagentId=ag_1  + text

transcript_patch                 parent user steer   (send-parent)

subagent_transcript_patch        subagentId=ag_1  + text (final)
subagent closed                  phase=completed  result=...
transcript_patch                 parent tool_call completed, stdout=result
```

## Product rendering

Products already render parent `tool_call` blocks. Nested tool use is the same
blocks on `subagent_transcript_*`, keyed by `subagentId` under the matching
`subagent started` job.

Root assistant text is the only user-visible product reply stream. Child `text`
blocks are for nested UI (cards, inspect), not a second user-facing reply.

## Layering

| Package | Role |
|---|---|
| `@demicodes/shell` | Foreground registered commands (signal, live IO, stdin stream) |
| `@demicodes/agent` | Supervisor, `demi agent` injection, protocol frames, child `AgentSession` |
| `@demicodes/coding-agent` | Optional named profiles (`explore` read-only Host, `default`) |
| harness / product | Extra profiles, Host wrapping, UI over `AgentClient` |

`@demicodes/coding-agent` does not instantiate `AgentSession`.

## Non-goals

- New model-facing tools
- Runtime `--model` / provider picker
- `clone()` as spawn
- Child checkpoints or `listConversations` entries
- Nested spawn (`DEMI_SUBAGENT_DEPTH >= 1`)
- `demi agent wait` / `result` (blocking spawn, `shell_status` / `yield`, and
  natural session end already cover these)
- Tailing the child transcript into spawn stdout or the parent inference
  transcript

## Coverage

- `packages/agent/src/__tests__/subagent.test.ts` — spawn isolation with the
  child preamble on an empty transcript, depth (child tree is send-parent
  only), the live-children ceiling, abort via `demi agent abort`, send-parent
  steer while the parent is blocked in that spawn, idle parent wakeup on
  completion, empty prompt fails, empty last text exits 0, `subagent*`
  protocol frames, inherited vs replaced `systemPrompt` with unknown-profile
  rejection, list/show snapshot bounds and finished-id miss, parent close
  aborts live children
- `packages/shell/src/__tests__/foreground-command.test.ts` — registered
  command abort signal, live stdout, `shell_write` as stdin stream, byte-clean
  pipes around a virtual foreground job
- `packages/coding-agent/src/__tests__/coding-harness.test.ts` — `default` /
  `explore` profiles and the injected `demi agent` prompt-field help
