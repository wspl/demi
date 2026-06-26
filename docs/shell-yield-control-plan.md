# Shell Control Surface + Yield Wakeup Design

| | |
|---|---|
| Date | 2026-06-26 |
| Status | Design (rewrite) |
| Scope | The `@demi/shell` control surface, `@demi/agent` delayed yield wakeups, and the `@demi/core` invisible user_message |

This document is the **single source of truth** for the shell control surface and the
yield wakeup mechanism. `docs/tool-rendering-spec.md` only references it and does not
restate these semantics.

## 1. Conclusion

Demi splits shell control and delayed wakeup into **two orthogonal concepts** that do
not depend on each other:

- **Shell control surface**: `shell_exec` / `shell_status` / `shell_write` / `shell_abort`.
  These express only "do something to a command" — they have **nothing to do with turns,
  timing, or wakeups**.
- **Yield wakeup**: `yield`. The single cross-turn delayed-wakeup mechanism. It expresses
  only "end the current turn and wake the session later with an invisible user_message" —
  it has **nothing to do with the shell**.

The model still sees exactly five tools:

```text
shell_exec
shell_status
shell_write
shell_abort
yield
```

Two key semantics (the two misunderstandings this rewrite corrects):

1. **`shell_exec` does not yield.** It has a `timeoutMs`, which is the **upper bound on a
   synchronous observation window**, not a kill deadline. When it elapses the process is
   not terminated, the command record is not released, the tool returns
   `running + commandId`, and **the current turn continues**. The model decides what to do
   next: `shell_status` immediately, `yield` for a while and look again, `shell_write`, or
   `shell_abort`. `shell_exec` never ends a turn on its own.

2. **A `yield` wakeup is delivered in only two ways — steer / send — both carrying the same
   invisible user_message.**
   - **steer**: when the session is **running**, insert this user_message into the current
     active turn (interjection).
   - **send**: when the session is **idle**, send this user_message to the session as a new
     turn (the ordinary `send` path, just with a `hidden` user_message). **This is not the
     `resume` used after abort/compaction — do not conflate the two.**

   This user_message is visible to the model (replayed normally) and not rendered to the user.

`shell_abort` is the only entry point that terminates a command. When `timeoutMs` elapses
nothing is terminated and nothing is implicitly killed; if a command seems stuck, the model
or user terminates it explicitly with `shell_abort`.

## 2. Motivation

### 2.1 What the old design got wrong

The old design had two conflations that must be cleaned up along with the implementation:

- **It wrapped the observation window as `yieldAfterMs` and forced a manual `shell_exec →
  yield` chain.** The parameter was named "yield after Ms" but the behavior was only "return
  running when it elapses, the turn continues, and rely on the model to add another
  `yield`." The name promised an automatic yield while the semantics were an observation
  window, and the model had to write two steps for every long command. Shell control and
  yield were wrongly coupled.

- **The wakeup payload was inconsistent across the two paths.** The active path turned the
  wakeup into a **visible steer block** (disguising an internal framework instruction as a
  user utterance), while the idle path pushed an empty resume turn and discarded the wakeup
  text entirely. The two paths gave the model different things, and neither was "one
  invisible user_message."

### 2.2 Principles after the rewrite

- **Orthogonal.** Shell tools never touch turns / timing / wakeups; `yield` never touches
  the shell / process / command record. Their only point of contact is that the model may
  **choose** to schedule a later re-check with `yield` while a shell command is still running.
- **A single wakeup payload.** Every yield wakeup delivers the same invisible user_message;
  steer / send is only the difference between "insert into a running turn" and "start a new
  turn on an idle session," not two different contents.
- **`timeoutMs` is only a synchronous wait bound.** It decides how long `shell_exec` waits
  inline before returning a running handle — it never terminates the process.

## 3. Tool semantics

### 3.1 `shell_exec`

Start a shell script and observe it synchronously within `timeoutMs`.

```ts
type ShellExecInput = {
  script: string
  shellId?: string
  timeoutMs: number
  description?: string
}
```

Semantics:

- `timeoutMs` is required, in the range `1..600000` (max 10 minutes). It is the **upper
  bound on the synchronous observation window**; when it elapses the process is **not
  terminated** and the command record is not released.
- `timeoutMs` is a pure per-call parameter, decided by the model on each `shell_exec` call:
  there is **no global or configurable default**, and no CLI flag or `BashEnvironment`
  option can change it. Only direct, non-model `exec()` paths (internal transient commands
  like `editor` / `todo`) fall back to a fixed internal constant when they omit it.
- If `shellId` is given, the command runs in that shell session.
- If `shellId` is omitted, the current agent session's default shell is used; an auxiliary
  shell is created when the default shell is running a foreground command.
- If the caller explicitly names a `shellId` that is running a foreground command, an error
  is returned with the current `commandId` — the running command is not overwritten.
- Command exits within `timeoutMs` with a complete preview: returns `status: "exited"`,
  `exitCode`, and the preview, does not expose a `commandId`, and releases the command record.
- Command exits within `timeoutMs` but the preview is truncated: returns `status: "exited"`,
  `exitCode`, the preview, and artifact paths, keeping the artifact read path.
- Command still running after `timeoutMs`: returns `status: "running"`, `commandId`, the
  preview, and artifact paths. **The command keeps running; the current turn does not end.**
- `shell_exec` itself **never ends a turn, schedules a wakeup, or terminates a process**.

`running` is a decision point for the model, not automatic framework behavior. The model may:

- `shell_status(commandId)` immediately to look again;
- `yield(durationMs)` to end the current turn and be woken later to `shell_status`;
- `shell_write(commandId, stdin)` (when it knows the command is waiting for input);
- `shell_abort(commandId)` (when it seems stuck or is no longer needed).

The tool result includes an output preview auto-budgeted to the current model context
window, so the model can quickly judge ordinary output, an obvious failure, or completion.
Ordinary short commands must be judged straight from the preview and should not re-read
`/@`. Only a truncated preview, a long-running command whose history must be inspected, or a
need for text search calls for the `/@` artifact.

### 3.2 `shell_status`

Read the current state of a running command handle. It does not wait or write stdin.

```ts
type ShellStatusInput = {
  commandId: string
  description?: string
}
```

Semantics:

- `commandId` points at a foreground command, not a shell session.
- Still running: returns `status: "running"`.
- Already exited: returns `status: "exited"`, `exitCode`, and a budgeted preview; releases
  the command record after returning when the preview is complete.
- Already aborted: returns `status: "aborted"`; releases the command record after returning
  when the preview is complete.
- Returns `runningMs`, `idleMs`, stdout/stderr byte counts, and artifact paths, so the model
  can judge whether there is new output or unusual silence.
- The new budgeted preview it returns exists only so the model can judge the result directly
  when the command completes or produces a little new output — it is **not an output
  pagination interface**.
- A `commandId` that has completed with a complete preview is no longer valid; only a
  truncated preview keeps the artifact read path.

### 3.3 `shell_write`

Write stdin to a foreground command; only non-empty input is accepted.

```ts
type ShellWriteInput = {
  commandId: string
  stdin: string
  description?: string
}
```

Semantics:

- `stdin` must be non-empty; polling must use `shell_status` — there is no empty-input
  compatibility path.
- After writing to the target command's stdin it immediately returns a status snapshot and a
  new budgeted preview; if the command thereby completes with a complete preview, the command
  record is released after returning.
- Returns an error when the command does not exist, has ended, or has no writable stdin.
- Whether to wait for output after the write is up to the model, by scheduling a follow-up
  turn with `yield` and calling `shell_status` after the wakeup.

### 3.4 `shell_abort`

Explicitly terminate a foreground command. This is the **only** tool that terminates a command.

```ts
type ShellAbortInput = {
  commandId: string
  description?: string
}
```

Semantics:

- Terminates the foreground process group the command runs in.
- Returns the final status snapshot and artifact paths.
- When called on a command that has already ended, returns that command's final status; it
  does not re-kill the shell session.
- `shell_abort` is a control action and does not by default signal agent task failure.

### 3.5 `yield`

End the current turn and schedule the runtime, after the turn completes, to wait a while and
then wake the session with an **invisible user_message**.

```ts
type YieldInput = {
  durationMs: number
  description?: string
}
```

Semantics:

- `durationMs` is required, in the range `1..600000` (max 10 minutes).
- `yield` **does not read the shell, write the shell, manage processes, or hold a
  commandId**. It is a pure agent-level mechanism.
- The `yield` tool result is a terminal result: once written to the transcript the current
  provider continuation ends; sampling does not continue in the same turn.
- `durationMs` is timed from **after the current turn completes and the session enters a
  waitable state**, not from the tool call.
- When it elapses, an invisible user_message is delivered (see §4):
  - session **idle** → **send**: start a new turn with this user_message (the ordinary
    `send` path, not an abort `resume`).
  - session **active** → **steer**: insert this user_message as an internal steer into the
    current active turn, reusing the user-steer insertion-point semantics — **never queued**.
- During the wait the user may freely send a new topic; this does not automatically cancel
  the pending wakeup.
- `yield` is a one-shot delayed wakeup; there is no `repeat` / `start_yield` / `stop_yield`.

The combination of `yield` and the shell is expressed entirely by the model; the framework
never chains them automatically:

```text
shell_exec(..., timeoutMs)        # running + commandId
yield(durationMs)                 # ends the turn

# wakeup turn / wakeup steer
shell_status(commandId)
yield(durationMs)

# next wakeup
shell_status(commandId)
```

## 4. The yield wakeup mechanism

A wakeup has three steps, and the timing is the heart of the mechanism:

1. **The current turn ends naturally.** The terminal `yield` tool result ends the current
   provider continuation and the turn settles to idle. `yield` does not preempt or interrupt
   ongoing sampling or tools; it simply lets the round finish normally.
2. **Timing starts only after the turn completes.** The timer is armed only after the turn
   has fully ended and the session has entered a waitable state — not from the moment `yield`
   was called. So "wait `durationMs`" means "wait `durationMs` after the turn ends."
3. **Deliver the invisible user_message when it elapses.** The delivery method is chosen by
   the session state at that moment:
   - **idle → send**: send this user_message to the idle session as an ordinary `send`,
     starting a new turn (just with a `hidden` user_message). **This is `send`, not the
     `resume` after abort/compaction; `resume` is a different mechanism — do not mix them.**
   - **active → steer**: insert this user_message as an internal steer into the current
     active turn, visible to the model at the nearest provider / tool boundary, **never
     entering the queue**.

The wakeup payload is **the same invisible user_message** regardless of delivery method:

- It is visible to the model — replayed normally as a `user_message` (send path) or
  `user_steer` (steer path), and the model reasons from it.
- It is not rendered to the user — the UI / REPL does not show it as a user utterance (see §5).
- Its content is a **shell-agnostic internal hint**, e.g. "the wait you scheduled earlier is
  over; continue your previous work, and use `shell_status` if a command is still running."
  `yield` knows no commandId; the model knows what it is waiting for from context.

Why a wakeup must not enter the queue: queuing it would let a long-command re-check be
deferred indefinitely behind the user's new topics, defeating yield's purpose. When active it
must be inserted into the current turn as a steer; when idle it is itself a new turn.

## 5. The invisible user_message

The wakeup payload needs a first-class "visible to the model, not rendered to the user" user
input.

The core layer adds a `hidden` flag to the two kinds of user input — user / steer:

```ts
type Block =
  | { type: 'user'; id: string; turnId: string; createdAt: string; model: ModelSelection
      content: UserContentBlock[]; preamble: string | null; hidden?: boolean }
  | { type: 'steer'; id: string; turnId: string; createdAt: string; model: ModelSelection
      content: UserContentBlock[]; hidden?: boolean }
  // all other blocks unchanged
```

Rules:

- `hidden` defaults to `false`. Ordinary user input carries no such flag and its behavior is
  entirely unchanged.
- `collectInferenceItems()` treats `hidden` and non-`hidden` identically: it still emits
  `user_message` / `user_steer`, so **the model always sees the wakeup input**. `hidden` does
  not enter provider replay and does not affect the turn grouping of compaction / retry / resume.
- The render layer (the Web UI's visible-block filter, the REPL renderer) **skips** user /
  steer blocks with `hidden === true` and does not show them as user bubbles.
- Yield wakeup delivery:
  - idle → start a new turn with a `hidden: true` user input (`pushUserTurn(..., { hidden: true })`).
  - active → insert a `hidden: true` steer (`pushSteer(..., { hidden: true })`).

`hidden` only affects whether the UI renders — that is its sole effect; it does not create a
second replay semantics, nor change turn boundaries.

## 6. The identifier model

`shellId` and `commandId` must be kept separate:

- `shellId`: a long-lived shell session handle, carrying cwd, env, functions, background jobs,
  and the current foreground command.
- `commandId`: a single foreground command handle, carrying process state, the stdout/stderr
  artifacts, byte counts, and exit info.

A shell session has at most one foreground command at a time. Background jobs remain shell
session state, but every long-running foreground task the model can observe and control uses a
`commandId`.

Default-shell rules:

- Every AgentSession has one default shell.
- When the default shell is idle, a `shell_exec` without `shellId` reuses it.
- When the default shell is busy, a `shell_exec` without `shellId` creates an auxiliary shell,
  so the model can run one-off check commands while a dev server is running.
- When an explicit busy `shellId` is passed, no auxiliary shell is created automatically,
  avoiding the model's mistaken belief that the command ran inside the named shell's state.

`commandId` belongs to the shell control surface; `yield` does not hold it. After a yield
wakeup the model calls `shell_status` with the `commandId` it remembers.

## 7. stdout/stderr artifacts

Every command has two read-only, append-only output artifacts and a metadata file:

```ts
type ShellArtifactRef = {
  path: string
  bytes: number
}

type ShellCommandSnapshot = {
  status: 'running' | 'exited' | 'aborted'
  shellId: string
  commandId: string
  stdout: ShellArtifactRef
  stderr: ShellArtifactRef
  runningMs: number
  idleMs: number
  exitCode?: number
  preview?: {
    text: string
    budgetTokens: number
    truncated: boolean
  }
}
```

Paths use the shell virtual-filesystem namespace:

```text
/@/commands/<commandId>/stdout.txt
/@/commands/<commandId>/stderr.txt
/@/commands/<commandId>/meta.json
```

For a retained command artifact, `stdout.txt` / `stderr.txt` keep their respective streams;
`meta.json` exposes status, exitCode, runningMs, idleMs, bytes, and timestamps. These files are
not written into the task cwd and do not pollute the user workspace. An ordinary short command
that completes with a complete preview is not retained; only running, preview-truncated, or
search-needing commands keep the full output as a durable audit sink. When a session snapshot is
saved or a session is restored, the runtime must be able to rebuild the still-needed
`/@/commands/<commandId>/...` from the durable command artifacts. The model-visible tool result
in the transcript prefers to store the auto-budgeted preview, storing a reference only when an
artifact must be retained; the UI/runtime may store interleaved output events for display, but
the interleaved stdout/stderr terminal transcript is not saved as a `/@` file.

Artifact content is the **model-visible stdout/stderr after shell redirection**, not raw process
fds:

- `cmd > file` should not leak stdout into the stdout artifact.
- `cmd 2>/dev/null` should not leak stderr into the stderr artifact.
- `cmd >&2` should go into the stderr artifact.
- The target file of `cmd > file` is still written through `Host.fs` per shell semantics.

`/@` is overlaid onto `HostBackedFileSystem` via just-bash's `IFileSystem` and is a read-only
virtual namespace:

- Portable commands like `cat` / `head` / `tail` / `grep` / `rg` / `sed` / `awk` / `wc` / `cut`
  / `sort` / `jq` reading `/@` paths must go through the just-bash command registry and the
  virtual FS.
- A real host external process cannot see the in-memory `/@` paths; a text-read command that
  includes a `/@` path must not fall back to host coreutils. When it cannot be executed via a
  portable path it should error explicitly.
- `/@` artifacts are read-only by default; any write, delete, rename, chmod, or link operation
  should be rejected.
- Only a retained command artifact's lifecycle follows the AgentSession's durable history; an
  ordinary short command that completes with a complete preview releases the command record and
  no longer exposes `/@`. Closing a live session releases the in-memory overlay, but a saved
  session snapshot must retain the artifact content or a restorable reference for anything that
  still needs reading.

Output sinks stream as they write: visible stdout/stderr chunks append to the matching command
artifact in real time; the file-redirection sink also writes to `Host.fs` chunk by chunk, so a
long command's target file is visible while it runs.

The model reads an artifact with an ordinary shell text command only when the preview is
truncated, a long-running command's history must be inspected, or text search is needed:

```bash
tail -n 80 /@/commands/<commandId>/stdout.txt
grep -n "ERROR" /@/commands/<commandId>/stderr.txt
sed -n '200,260p' /@/commands/<commandId>/stdout.txt
awk '/failed|error/i { print NR ":" $0 }' /@/commands/<commandId>/stderr.txt
```

`maxOutputBytes` is not part of the final model-visible schema. The output budget is decided
automatically by `@demi/agent` from the current model's `contextWindow`; it affects only the
tool-result preview, not artifact storage:

| Model context window | Tool-result preview budget |
|---|---:|
| Unknown or `< 800_000` tokens | `1_000` tokens |
| `>= 800_000` tokens | `10_000` tokens |

The budget unit is tokens. The implementation prefers the provider/model tokenizer; with no
tokenizer it uses a conservative estimate to convert `budgetTokens` into a character limit. A
truncated preview must carry `truncated: true` and prompt the model to read the needed part via
`/@/commands/<commandId>/...`; a complete preview does not expose artifact paths and so does not
invite a second read.

## 8. Agent loop behavior

The agent loop resumes the model only at well-defined boundaries:

- `shell_exec`'s `timeoutMs` elapses (the tool result returns, **the turn continues**).
- A command exits.
- A `shell_status` / `shell_write` / `shell_abort` tool result returns.
- A `yield` tool result returns and **ends the current turn**.
- A pending yield wakeup elapses: when idle, start a new turn with the invisible user_message;
  when active, insert it as an internal steer into the current turn.
- The provider stream itself completes or errors.

The arrival of an output chunk does **not** wake the model directly. The UI may display terminal
output live from the artifact or progress events; the model only resumes reasoning after a tool
returns, an internal wakeup turn, or an internal wakeup steer.

Slow output and very large output share one mechanism: the model lets a long command run in the
foreground, and `shell_exec` returns `running` at `timeoutMs`; the model `yield`s as needed and,
after waking, uses `shell_status` to judge whether the command is still running, has new bytes,
or has been idle too long; ordinary new output is judged straight from the budgeted preview; only
a truncated preview or a need for search calls for reading the `/@` artifact with `tail` / `grep`
/ `awk` / `sed`. The runtime never wakes the model automatically because output is slow or large,
and never stuffs a large chunk of output into `shell_status`.

## 9. Typical flows

### A long test command

```text
Turn A
shell_exec({ script: "pnpm test", timeoutMs: 10000 })
→ running + commandId (turn not ended)

yield({ durationMs: 30000 })
→ scheduled, Turn A completes

Turn B (internal wakeup 30s later, sends an invisible user_message)
shell_status({ commandId })
→ running, output bytes growing

yield({ durationMs: 30000 })
→ scheduled, Turn B completes

Turn C (internal wakeup 30s later)
shell_status({ commandId })
→ exited + exitCode

tail -n 80 /@/commands/<commandId>/stdout.txt
grep -n -E "ERROR|FAIL" /@/commands/<commandId>/stderr.txt
```

### Dev-server smoke check

```text
shell_exec({ script: "pnpm dev", timeoutMs: 3000 })
→ running + commandId=server

shell_exec({ script: "curl -I http://127.0.0.1:18922", timeoutMs: 10000 })
→ exited (default shell busy, an auxiliary shell is used automatically, without
  interrupting the dev server)

shell_abort({ commandId: server })
```

### Interactive input

```text
Turn A
shell_exec({ script: "node prompt.js", timeoutMs: 1000 })
→ running + commandId

shell_write({ commandId, stdin: "Alice\n" })
yield({ durationMs: 500 })
→ scheduled, Turn A completes

Turn B (internal wakeup 500ms later)
shell_status({ commandId })
→ running

tail -n 40 /@/commands/<commandId>/stdout.txt
```

### Suspected hang

```text
shell_status({ commandId })
→ running, idleMs very large

shell_abort({ commandId })
→ aborted + last output
```

### The user starts a new topic during the wait

```text
Turn A
shell_exec({ script: "pnpm test", timeoutMs: 10000 })
→ running + commandId
yield({ durationMs: 30000 })
→ scheduled, Turn A completes

Turn B (user sends a new topic 10s later)
...the model is handling the user's new topic (session active)...

30s elapses
→ the runtime inserts the wakeup user_message into Turn B as an internal steer
  (not rendered, not queued)
→ the model sees it at Turn B's nearest provider/tool boundary, then decides whether
  to shell_status({ commandId })
```

## 10. The `abort` hierarchy

Here `abort` is the AgentSession / AgentClient session-control action, not the model-visible
`shell_abort`.

`abort` is a repeatable, layered settling action. Each call cancels only the current
highest-priority layer that can still be cancelled, and returns what it cancelled plus whether
`abort` can continue:

```ts
type AbortTarget =
  | 'active_provider_stream'
  | 'active_tool'
  | 'active_compaction'
  | 'active_turn'
  | 'queued_message'
  | 'queued_action'
  | 'pending_yield_wakeup'
```

Priority, highest to lowest:

1. The current active provider stream, reference resolution, compaction, or tool invocation.
2. The current active turn's remaining settling state.
3. A queued action awaiting execution — send / retry / resume / compact, etc.
4. A pending `yield` wakeup.

A pending `yield` wakeup is the lowest priority. An ordinary active-turn abort should not also
clear the wakeup, because it may be a schedule left by a previous long-command re-check. Only
when there is no active work, no queued action, and the caller aborts again is the pending
wakeup cancelled. Closing the session clears all pending wakeups outright.

`shell_abort(commandId)` does not go through this hierarchy; it is the tool with which the model
or user explicitly terminates a foreground command. Unless the currently executing shell tool
invocation is itself aborted, AgentSession `abort` should not implicitly terminate a shell
command that has already returned `running`.

## 11. Why there is no repeat yield

`yield({ repeat: true })` would mix two different mechanisms:

- A timed wakeup after a single turn ends: `yield`.
- Long-lived repeated wakeups: heartbeat / automation.

If every wakeup hands control back to the model, it is equivalent to the model explicitly
calling a one-shot `yield` again in each wakeup turn. So `yield` stays one-shot; long-lived
automatic polling will use a separate heartbeat design later.

## 12. Package responsibilities

The five model-visible tools are one complete agent base toolset, not split across packages by
tool name:

- `@demi/core` owns the shared `Block` types, including the `hidden` flag on user / steer.
- `@demi/agent` owns the model-visible tool surface: the names, schemas, and tool-call/result
  transcript semantics of `shell_exec` / `shell_status` / `shell_write` / `shell_abort` /
  `yield`, plus the post-turn delayed-wakeup scheduling, the idle-send invisible-user_message
  turn, the active-steer invisible user_message, and the explicit-abort / session-close cleanup.
  `yield`'s wakeup payload is constructed by `@demi/agent` and does not depend on the shell.
- `@demi/shell` owns the shell runtime services: BashEnvironment, shell sessions, command
  records, command artifacts, the Host-backed stream sink, and the exec/status/write/abort
  primitives. It **owns no model-visible AgentTool, and knows nothing of `yield`, turns, or
  wakeups**.
- `@demi/coding-agent` only explains the usage policy for these five tools in the prompt.
- `@demi/web-ui` and `@demi/repl` consume the unified protocol events to display state, and skip
  `hidden` user/steer blocks per §5.

So the final layering is: `@demi/agent` owns the complete tool surface and the yield wakeup
semantics, `@demi/shell` provides only shell execution, and the two are orthogonal.

## 13. Tests & acceptance

Unit-test modules and their intended coverage:

- `packages/agent/src/tools.ts` (covered by agent/coding/provider integration tests)
  - The tool schema exposes only `shell_exec` / `shell_status` / `shell_write` / `shell_abort`
    / `yield`.
  - `shell_exec` exposes `timeoutMs` (required, max 10 minutes) and **does not expose
    `yieldAfterMs`**.
  - `shell_write` rejects empty stdin.
  - The tool result contains only status, exitCode, and preview for an ordinary completion;
    `shellId`, `commandId`, artifact paths, and a next action appear only when running or the
    preview is truncated.
  - The final schema exposes neither `maxOutputBytes`, stdout/stderr offsets, nor a
    model-controlled output budget.
  - When `shell_exec` returns `running` it carries no implicit "end the turn / schedule a yield"
    semantics; `yield` and the shell tools are independent schemas.

- `packages/shell/src/__tests__/environment.test.ts`
  - `shell_exec` past `timeoutMs` returns `running` and **does not kill the process or release
    the record**.
  - `timeoutMs` is the synchronous observation-window bound, not a kill deadline.
  - `shell_status` reads state and a new budgeted preview non-blockingly, and does not do output
    pagination.
  - After an ordinary short command completes, the command record is released and subsequent
    status and `/@` reads both fail.
  - A retained command artifact exposes `/@/commands/<commandId>/stdout.txt`, `stderr.txt`, and
    `meta.json`.
  - `tail` / `grep` / `sed` / `awk` / `wc` can read `/@` artifacts via just-bash portable
    commands; a text-read command containing `/@` does not fall back to a host external process.
  - The final artifact is still readable after the command exits.
  - `shell_abort` terminates the foreground process group and retains the final output.
  - Redirection does not leak into the visible stdout/stderr artifacts; the file-redirection sink
    is visible while a long command runs.
  - When the default shell is busy, a `shell_exec` without `shellId` creates an auxiliary shell;
    a named busy `shellId` is rejected.

- `packages/agent/src/__tests__/session.test.ts`
  - The `yield` tool result is a terminal result; the current provider continuation stops
    sampling and the turn ends naturally.
  - The `yield` duration is timed from **after** the current turn completes.
  - When it elapses with the session idle: a new turn starts with the invisible user_message
    (send, not an abort `resume`), the model sees that user_message on replay, and can go on to
    call `shell_status`.
  - When it elapses with the session active: the invisible user_message is inserted into the
    current active turn as an internal steer and does not enter the queue.
  - The wakeup user_message's `hidden` is true: `collectInferenceItems()` still emits
    `user_message` / `user_steer` (model-visible), and the render layer filters it (user-invisible).
  - `yield` neither reads nor writes shell state, holds no commandId, and does not implicitly
    abort a shell command.
  - `abort` settles layer by layer in priority order — active work, queued action, pending yield
    wakeup; an active-work abort does not clear a pending yield wakeup; a pending wakeup is
    cleared only at the last abort layer or on session close.
  - Neither slow nor very large output wakes the model chunk by chunk.
  - The tool-result preview budget is chosen automatically from the current `Model.contextWindow`:
    1k tokens when unknown or below 800k, 10k tokens at 800k and above.

- `packages/agent/src/__tests__/server.test.ts`
  - AgentServer exposes the new tool surface and corresponding events to the client.
  - A pending yield wakeup is observable in the protocol state; idle elapse opens a new turn,
    active elapse sends an internal steer.
  - The abort response/frame exposes `target` and `canAbortAgain`.

- The render layer (`packages/web-ui`, `packages/repl`)
  - User / steer blocks with `hidden === true` are not rendered as user bubbles; non-hidden
    behavior is unchanged.

- Real-model acceptance
  - A real model monitors a long command to completion with `shell_exec(timeoutMs) → yield →
    shell_status`, and reads `/@` artifacts with `tail` / `grep` / `sed`.
  - A real model starts a dev server, verifies it with an auxiliary shell, then `shell_abort`s to
    clean up.

Acceptance criteria:

- A long command past 120 seconds is not killed by `timeoutMs`; `timeoutMs` only decides when
  `shell_exec` returns `running` synchronously.
- When `shell_exec` returns `running` the current turn continues, the model may `shell_status`
  immediately or `yield` on its own; the framework does not chain yield automatically.
- A yield wakeup delivers the same invisible user_message whether idle or active; the model
  always sees it and the user never does.
- A user's new topic does not push the pending yield into the queue; the elapsed wakeup enters
  the active turn as an internal steer and does not implicitly abort the shell command.
- `abort` is repeatable; a pending yield wakeup is the lowest priority.
- Every command-termination path shows an explicit `shell_abort` in the transcript.
- After a session is restored, a historical command's `/@/commands/<commandId>/stdout.txt`,
  `stderr.txt`, and `meta.json` are still readable, with content identical to the original
  complete visible output.
