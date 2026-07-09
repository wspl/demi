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

No product-side socket/shim assembly.

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

---

## Architecture

```text
createLocalAgentServer
  stateDir = resolveDemiHome(options.stateDir)
  socket   = stateDir/bridges/<id>.sock
  AgentServer.commandBridge = { socketPath, shimSource, stateDir }
  startCommandBridge(server)
  open() → materialize stateDir/bridge-bin/<sessionId>/
```

---

## Package responsibilities

| Package | Role |
|---------|------|
| `@demicodes/shell` | Command tree, exec, quoting. |
| `@demicodes/agent` | Low-level `commandBridge: { socketPath, shimSource, stateDir }`; materialize; `runCommandLine`. |
| `@demicodes/agent/command-bridge` | Shim source + UDS listener. |
| `@demicodes/host-local` | `LocalHost`, `resolveDemiHome`, `createLocalAgentServer`. |

---

## Protocol / exec / stdin

- UDS `POST /run` → `{ commandScopeId, name, args, cwd, stdin }` → `{ exitCode, stdout, stderr }`
- Exec: `cd … && name args…` (+ heredoc if stdin non-empty)
- Stdin: 300ms grace to first byte, then read to EOF

---

## Key decisions

| Decision | Choice |
|----------|--------|
| State root | `~/.demi` / `$DEMI_HOME` / `stateDir` |
| Shim path | **Fixed** `<stateDir>/bridge-bin/<sessionId>` |
| Custom bin dir | **Not supported** |
| Socket | `<stateDir>/bridges/<id>.sock` (path override only) |
| Default bridge | **On** for `createLocalAgentServer` |
