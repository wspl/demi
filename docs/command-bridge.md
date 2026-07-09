# Command Bridge

Final-state design: make every top-level registered `Command` reachable as a normal OS executable from subprocesses spawned inside an agent session’s shell tree.

Depends on the recursive `Command` model in `@demicodes/shell` (single node type; routing and execution independent). Bridge does not reimplement command parsing or execution.

---

## Problem

A session’s registered commands are installed into the **in-memory** just-bash interpreter for a given `BashEnvironment.exec` call. That map is not on `PATH` and is not visible to a real OS process.

| Invocation | Sees registered commands? |
|------------|---------------------------|
| Model / tool: `shell_exec` with script `editor create …` | Yes — same interpreter |
| Real subprocess: `node` / `python` / nested `bash` then `execve("editor", …)` | No — only OS `PATH` |

Products need nested tooling (scripts, CLIs, eval runners) to call the same capability surface the model uses, without a second registration system.

---

## Goals

1. **One capability surface** — only `Command` / `CommandRegistry`. No bridge-specific capability API.
2. **Same execution** — bridge invocations run through the same path as a typed shell line ending in a registered command (audit, storage, stdin field, assets, exit code).
3. **Whole registry** — every top-level root name is exposed automatically; no per-command wiring.
4. **Session-scoped** — a call is bound to one live `agentSessionId` and that session’s environment and storage.
5. **Full I/O for the caller** — subprocess callers get complete stdout/stderr, not model token-budget previews.
6. **Opt-in assembly** — products enable the bridge explicitly; platform-neutral agent root does not start OS listeners by default.

## Non-goals

- Streaming request/response bodies over the bridge.
- Surfacing each nested bridge call as a model-visible tool turn (audit/artifacts remain; transcript tool_call does not).
- A second permission model beyond “process already runs inside the session’s host sandbox.”
- Multiplexed transports (HTTP/2, shared long-lived streams) for call concurrency.
- Replacing `shell_exec` for the model; the model path stays shell script + fork map.

---

## Core idea

```text
┌─────────────────────────────────────────────────────────────┐
│ Agent process                                                 │
│                                                               │
│  CommandRegistry (roots: editor, todo, …)                     │
│         │                                                     │
│         ├─ shell_exec ──► BashEnvironment (fork map) ──┐      │
│         │                                              │      │
│         └─ bridge: runCommandLine ─────────────────────┤      │
│                        ▲                               ▼      │
│                        │                    runRegisteredCommand
│              UDS HTTP  │                               │      │
│                        │                               ▼      │
│              listener ─┘                         command.run  │
└───────────────────────────┬───────────────────────────────────┘
                            │ unix socket
┌───────────────────────────▼───────────────────────────────────┐
│ Session sandbox (Host.fs + Host.process)                      │
│                                                               │
│  PATH=.demi-bin/<sessionId>:…                                 │
│  DEMI_COMMAND_BRIDGE_SOCK=…                                   │
│  DEMI_AGENT_SESSION_ID=<sessionId>                            │
│                                                               │
│  node script ──spawn──► editor create foo  (symlink → shim)   │
│                               │                               │
│                               └── POST /run { name, args, … } │
└───────────────────────────────────────────────────────────────┘
```

Bridge is a **PATH adapter + in-process dispatch**, not a new command runtime.

---

## End-to-end contract

### What a subprocess experiences

```bash
# inside a process started under the session shell (PATH already set)
editor create path/to/file.ts <<'EOF'
// content
EOF
echo $?   # command exit code
```

Behavior matches the same argv/stdin as:

```bash
# typed into shell_exec
editor create path/to/file.ts <<'EOF'
// content
EOF
```

Nested paths are ordinary argv after the root name (`larkclaw watch create …`). Bare roots (`kcenv KEY`) work the same way. The bridge never walks the command tree itself.

### What the product wires

1. Construct `AgentServer` with a harness that registers commands (as today).
2. Start the bridge listener against that server (Node-only entry).
3. Pass bridge options into the server so each `open()` materializes shims and injects env.

Without (2)–(3), behavior is unchanged: only in-interpreter registered commands.

---

## Shim materialization

### When

On every successful session `open()`, after the session’s `CommandRegistry` is built and before (or as) that session’s `BashEnvironment` / shell env is fixed for the lifetime of the open.

Cheap and **idempotent**: re-open or resume rewrites the same directory layout.

### Where

Under the session-visible filesystem root (same namespace as `Host.fs` and process cwd policy):

```text
.demi-bin/<agentSessionId>/
  package.json      # {"type":"commonjs"} — isolate module type from the workspace
  .dispatch         # single executable dispatch script (shim body)
  <rootName> -> .dispatch   # one symlink per top-level registry name
```

Only **root** names from `registry.list()` get symlinks. Nested commands are not separate PATH entries.

### PATH / env injection

For every shell environment belonging to that session open:

| Variable | Role |
|----------|------|
| `PATH` | Prepend the realpath of `.demi-bin/<agentSessionId>/` |
| `DEMI_COMMAND_BRIDGE_SOCK` | Absolute path of the process-wide UDS endpoint |
| `DEMI_AGENT_SESSION_ID` | Already used for command storage / scope; shim sends it as `commandScopeId` |

Assumption: `Host.fs` and `Host.process` share one real filesystem view for that workspace (true for `LocalHost`; required for any host that enables the bridge).

### Shim behavior (logical)

1. Resolve invoked name = basename of `argv[0]` / the path used to exec the symlink.
2. Read `DEMI_COMMAND_BRIDGE_SOCK` and `DEMI_AGENT_SESSION_ID` from the environment (hard fail if missing).
3. Collect `args = argv.slice(1)`, `cwd = process.cwd()`, `stdin =` full stdin as utf8 under the **stdin policy** below.
4. `POST /run` on the unix socket; write response stdout/stderr to the process streams; exit with `exitCode`.

One script body for all names; only symlink names change when the registry changes.

### Stdin policy (design, not a patch)

The shim must not hang forever when the parent never closes stdin, and must not silently truncate a parent that does stream input.

**Contract:**

- Read stdin with a **start grace** (fixed, documented constant): if **no byte** arrives within the grace window, treat stdin as empty and proceed.
- If **any** byte arrives within the grace window, clear the grace timer and read until EOF with **no time cap** (caller owns the stream lifetime).
- If bytes arrive **after** the grace window closed and the request was already sent with empty stdin, emit a clear diagnostic on **stderr** (and do not pretend the command received that data).

Products that need reliable large stdin should prefer writing a file and passing a path (existing convention), or ensure the parent closes stdin promptly after writing.

---

## In-process protocol

### Transport

- One **unix domain socket** listener per agent process (not per session).
- HTTP/1.1 request/response, **one connection per logical call**.
- Concurrency = OS connections; no application-level multiplexing.

### Request

`POST /run` body (JSON):

```ts
{
  commandScopeId: string  // agent session id
  name: string            // top-level command name
  args: string[]          // remaining argv
  cwd: string             // desired working directory
  stdin: string           // utf8; may be empty
}
```

### Response

- `200` `{ exitCode: number, stdout: string, stderr: string }`
- `4xx/5xx` `{ error: string }` (unknown session, bad body, internal failure)

No command id is required in the protocol for cancellation: connection close is the cancel signal for that call.

---

## Server-side dispatch

### Lookup

Resolve `commandScopeId` to the live binding that owns that session in this process (reuse the existing session ownership registry; do not invent a parallel map).

Missing / closed session → client error response, not process crash.

### Execution

1. Authorize only by “session is live in this process” (trust model below).
2. Build a single shell script line:

   ```text
   cd <quoted cwd> && <quoted name> <quoted args…>
   ```

   with stdin delivered the same way other shell invocations deliver stdin to registered commands (e.g. heredoc / pipe into that script — implementation detail must preserve exact stdin bytes including empty).

3. Call `BashEnvironment.exec` for that session with:
   - `agentSessionId = commandScopeId`
   - no forced `shellId` (same idle-default / auxiliary shell selection as `shell_exec`)
   - a **bridge ceiling** `timeoutMs` (sync wait bound for the blocked caller)
   - an `AbortSignal` tied to the HTTP connection lifetime

4. Return **complete** stdout/stderr from that run’s snapshot (not model preview windows). On ceiling expiry without completion: abort the command and return a timeout error.

### Cancellation

| Trigger | Behavior |
|---------|----------|
| Connection closed before response | Abort the in-flight `exec` (kill underlying work the same way session abort does). |
| Bridge wait ceiling elapsed | Abort by command id and respond with timeout. |

### Model visibility

The model sees only the outer tool (e.g. a `node` script’s output). Nested bridge runs still produce normal shell audit / command records / artifacts for operators.

---

## Trust and security

- A process that can read `DEMI_AGENT_SESSION_ID` and reach the UDS path is already inside the session’s execution environment.
- Bridge does **not** add new capabilities beyond invoking commands that session already could run via top-level shell.
- Socket filesystem permissions: owner-only (or host-equivalent) for the agent process user.
- Do not accept remote TCP for this protocol in the default design.

---

## Package responsibilities

| Package | Responsibility |
|---------|----------------|
| `@demicodes/shell` | `Command` tree, registry, `runRegisteredCommand`, quoting helpers, `BashEnvironment.exec` / abort / snapshots. No sockets. |
| `@demicodes/agent` (platform-neutral root) | Optional bridge config on the server; materialize shim directory via `Host.fs` on open; inject PATH/env; `runCommandLine` (or equivalent) on the server using session ownership + environment. Shim **source text** is not authored here if that forces Node-only APIs into the neutral package. |
| `@demicodes/agent/command-bridge` (Node-only subpath) | Owns shim script source; starts UDS listener; HTTP parse; calls into the server’s run API; process lifecycle of the listener. |
| Product / host assembly (e.g. web, REPL) | Creates socket path, starts listener, passes options into `AgentServer`. |

Dependency direction stays: shell ← agent ← products. Host-local gains nothing bridge-specific unless a future host needs custom socket placement.

---

## Configuration surface (conceptual)

```ts
// Platform-neutral server options (names illustrative)
commandBridge?: {
  /** Absolute filesystem path for the UDS endpoint. */
  socketPath: string
  /**
   * Dispatch script body for .dispatch (supplied by the Node entry
   * so the neutral package never embeds Node-only source).
   */
  shimSource: string
}
```

Node entry:

```ts
// @demicodes/agent/command-bridge
startCommandBridge(server, { socketPath }): { stop(): Promise<void> }
// exports SHIM_SOURCE constant used when constructing AgentServer options
```

Exact TypeScript names can match existing codebase style when implementing; the split of ownership must not.

---

## Failure modes (normative)

| Situation | Outcome |
|-----------|---------|
| Bridge not started; shim runs | Shim fails immediately (missing sock / connection refused). |
| Session already closed | `4xx` with explicit “session not found”. |
| Unknown root `name` | Same as unknown command in shell (non-zero exit + stderr message). |
| Invalid cwd | `cd` fails; command does not run. |
| Parse/validation error inside command | Non-zero exit + stderr from the command path. |
| Grace stdin race (late data) | Command may have run with empty stdin; late data reported on shim stderr. |

---

## Acceptance criteria

1. After open with bridge enabled, `PATH` contains the session shim dir; one symlink per root command name.
2. From a real Node (or other) child of `shell_exec`, `execFileSync('editor', ['create', …])` (or equivalent) produces the same filesystem/storage effects and exit code as the same line in top-level shell.
3. Nested argv (`… watch create …`) and bare roots both work without extra bridge logic.
4. Concurrent bridge calls against one session do not deadlock (auxiliary shells as with concurrent shell_exec).
5. Dropping the client mid-call aborts server-side work.
6. Timeout ceiling aborts and returns an error; caller is not blocked unbounded.
7. Full stdout/stderr returned to the subprocess (not model preview truncation).
8. Disabling bridge config leaves no PATH shim and no listener requirement.

---

## Implementation order

Single final-state slice preferred (no compatibility shims for an older bridge):

1. Server: session lookup + `runCommandLine` + tests with fake registry commands.
2. Materialize shim dir + PATH/env on open (inject script body from options).
3. Node subpath: listener + shim source + integration tests spawning a real child that bareword-invokes a registered command.
4. Product wiring (web/repl) only if those entrypoints should enable the bridge by default or via flag.

Do not land a second parallel bridge API.

---

## Key decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Capability API | Only `Command` | One surface for model and subprocesses |
| PATH entries | Top-level roots only | Nested structure is argv; matches CLI reality |
| Execution | Re-enter `BashEnvironment.exec` | Shared audit, storage, parse, abort |
| Transport | UDS + HTTP/1.1, one conn per call | Simple concurrency; enough for local IPC |
| Session index | Reuse ownership registry | No dual source of truth |
| Output | Full snapshot streams | Subprocess is not a model context window |
| Shim location | `.demi-bin/<sessionId>/` under Host.fs | No new Host facet; session-scoped cleanup unit |
| Node vs neutral | Listener + shim source on Node subpath | Keep agent root platform-neutral |
| Stdin | Grace-to-first-byte, then read to EOF | Avoid hang on never-closed stdin without truncating real pipes |

---

## Out of scope for this document

- Redesign of the `Command` tree (already specified by the implemented model).
- Native tool projection of commands.
- Remote multi-machine hosts that do not share a filesystem with the agent process (would need a different transport; not this design).
