# Command Bridge

Make every top-level registered `Command` available as a normal OS executable to subprocesses under a **local** agent session (`LocalHost` + Node).

Depends on the recursive `Command` model in `@demicodes/shell`. Bridge does not reimplement parse/run.

---

## Product shape (open box)

**Default: on** via `createLocalAgentServer` from `@demicodes/host-local`.

```ts
import { LocalHost, createLocalAgentServer } from '@demicodes/host-local'

const host = new LocalHost(cwd)
const { server, close } = createLocalAgentServer({
  host,
  agent: harness,
  providers,
  // commandBridge: true   ← default
  // commandBridge: false  ← only way to turn off
  // stateDir: '…'         ← optional; default ~/.demi or $DEMI_HOME
})
await close()
```

No product-side socket/shim assembly. Bin paths and UDS are **not** user-facing options — they are internal to LocalHost assembly (`stateDir` layout only).

---

## State layout (fixed under `stateDir`, not workspace cwd)

| | Default |
|--|---------|
| State root | `$DEMI_HOME` or `~/.demi` (`stateDir` option) |
| Socket | **always** `<stateDir>/bridges/<id>.sock` (or `commandBridgeSocketPath` override for the socket file only) |
| Command shims | **always** `<stateDir>/bridge-bin/<sessionId>/` — **not** separately configurable |

```text
~/.demi/                         # stateDir
  bridges/
    <serverId>.sock
  bridge-bin/                    # fixed name under stateDir
    <sessionId>/
      package.json
      .dispatch
      demi -> .dispatch
```

- Workspace **cwd** is only for project files / shell PWD.
- There is **no** `bridgeBinDir` option — bin root is fixed as `bridge-bin` under `stateDir`.

---

## Goals

1. Open box on LocalHost; bridge **on** by default.
2. State under home (or `stateDir`), not project cwd.
3. Fixed `bridge-bin` under stateDir.
4. Same `Command` execution path as in-shell registered commands.
5. Off switch: `commandBridge: false` only.
6. **AgentServer never knows about UDS or bin dirs** — only host-agnostic hooks/APIs.

---

## Architecture

```text
createLocalAgentServer (host-local)
  stateDir = resolveDemiHome(options.stateDir)
  socket   = stateDir/bridges/<id>.sock
  AgentServer({
    prepareShell → materialize stateDir/bridge-bin/<sessionId>/
                          inject PATH + DEMI_COMMAND_BRIDGE_SOCK
  })
  startCommandBridge(server) → UDS POST /run → server.runCommandLine(...)
```

### Layering

| Layer | Knows about |
|-------|-------------|
| `@demicodes/agent` `AgentServer` | `prepareShell` hook; `runCommandLine` for a live shell. **No** socket, shim, or bin layout. |
| `@demicodes/host-local` | UDS listener, `.dispatch` shim, `bridge-bin/`, `bridges/`, default-on assembly. |

`AgentServer` is intentionally transport-agnostic: another Host can supply a different `prepareShell` / IPC without AgentServer growing LocalHost concepts.

---

## Package responsibilities

| Package | Role |
|---------|------|
| `@demicodes/shell` | Command tree, exec, quoting. |
| `@demicodes/agent` | `prepareShell`; `runCommandLine` (shell-origin registered command exec). |
| `@demicodes/host-local` | `LocalHost`, `resolveDemiHome`, shim materialize, UDS bridge, `createLocalAgentServer`. |

---

## Protocol / exec / stdin

- UDS `POST /run` → `{ shellId, name, args, cwd, stdin }` → `{ exitCode, stdout, stderr }`
- The shim reads `shellId` from `DEMI_SHELL_ID`. It sends neither action metadata
  nor `agentSessionId`; the shell id locates the live binding and originating
  `BashEnvironment` directly.
- Exec: `name args…` (+ heredoc if stdin non-empty) via `AgentServer.runCommandLine`,
  always in an ephemeral shell born in the caller's cwd — bridge calls never
  share (or mutate) the model's session shell
- Stdin: 300ms grace to first byte, then read to EOF; newline-terminated stdin arrives
  byte-identical, stdin without a trailing newline gains exactly one (heredoc normalization)

---

## Key decisions

| Decision | Choice |
|----------|--------|
| State root | `~/.demi` / `$DEMI_HOME` / `stateDir` |
| Shim path | **Fixed** `<stateDir>/bridge-bin/<sessionId>` |
| Custom bin dir | **Not supported** (not user-facing) |
| Socket | `<stateDir>/bridges/<id>.sock` (path override only) |
| Default bridge | **On** for `createLocalAgentServer` |
| AgentServer bridge awareness | **None** — hook + `runCommandLine` only |

The current action may already use another Host when a child process calls the
bridge. Routing by the creating `shellId` keeps the invocation on its original
Host instead of following current action metadata.
