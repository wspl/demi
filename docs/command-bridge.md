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
      editor -> .dispatch
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
    prepareSessionShell → materialize stateDir/bridge-bin/<sessionId>/
                          inject PATH + DEMI_COMMAND_BRIDGE_SOCK
  })
  startCommandBridge(server) → UDS POST /run → server.runCommandLine(...)
```

### Layering

| Layer | Knows about |
|-------|-------------|
| `@demicodes/agent` `AgentServer` | `prepareSessionShell` hook; `runCommandLine` for a live session. **No** socket, shim, or bin layout. |
| `@demicodes/host-local` | UDS listener, `.dispatch` shim, `bridge-bin/`, `bridges/`, default-on assembly. |

`AgentServer` is intentionally transport-agnostic: a browser Host could supply a different `prepareSessionShell` / IPC without AgentServer growing LocalHost concepts.

---

## Package responsibilities

| Package | Role |
|---------|------|
| `@demicodes/shell` | Command tree, exec, quoting. |
| `@demicodes/agent` | `prepareSessionShell`; `runCommandLine` (session-scoped registered command exec). |
| `@demicodes/host-local` | `LocalHost`, `resolveDemiHome`, shim materialize, UDS bridge, `createLocalAgentServer`. |

---

## Protocol / exec / stdin

- UDS `POST /run` → `{ commandScopeId, name, args, cwd, stdin }` → `{ exitCode, stdout, stderr }`
- Exec: `cd … && name args…` (+ heredoc if stdin non-empty) via `AgentServer.runCommandLine`
- Stdin: 300ms grace to first byte, then read to EOF

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
