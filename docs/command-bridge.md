# Command Bridge

Make every top-level registered `Command` available as a normal OS executable to subprocesses under a **local** agent session (`LocalHost` + Node).

Depends on the recursive `Command` model in `@demicodes/shell`. Bridge does not reimplement parse/run.

---

## Product shape (open box)

**Default: on** when you use the local stack entry point.

```ts
import { LocalHost, createLocalAgentServer } from '@demicodes/host-local'
import { createCodingAgentHarness } from '@demicodes/coding-agent'

const host = new LocalHost(cwd)
const harness = createCodingAgentHarness({ host })
const { server, close } = createLocalAgentServer({
  host,
  agent: harness,
  providers,
  // commandBridge: true  ← default
  // commandBridge: false ← only way to turn off
})
// use server.attachTransport / server.client() as usual
await close()
```

Products (**web**, REPL, eval) should call `createLocalAgentServer` — **not** hand-wire socket paths, shim source, and listeners.

| API | Who | Default |
|-----|-----|---------|
| `createLocalAgentServer` | `@demicodes/host-local` | **bridge on** |
| `commandBridge: false` | same options | off |
| Low-level `AgentServer` + `@demicodes/agent/command-bridge` | tests / advanced | off unless configured |

---

## Problem

Registered commands live in the in-memory just-bash fork map for one `exec`. Real OS children (`node`, `python`, nested `bash`) only see `PATH` / `execve`, so bareword `editor` fails unless something bridges them back into the registry.

---

## Goals

1. **Open box on LocalHost** — one factory; bridge on by default.
2. **One capability surface** — only `Command` / `CommandRegistry`.
3. **Same execution** — bridge calls re-enter `BashEnvironment.exec` → `runRegisteredCommand`.
4. **No product splicing** — products do not assemble UDS + shim source.
5. **Explicit off switch** — `commandBridge: false` when a local product does not want it.

## Non-goals

- Streaming over the bridge.
- Model-visible tool turns for nested bridge calls (audit/artifacts remain).
- Remote Hosts without a shared real filesystem with the agent process.
- Pure in-browser Host runtimes (out of product scope for this feature).

---

## Architecture

```text
createLocalAgentServer (host-local)          [default: bridge ON]
        │
        ├─ AgentServer({ commandBridge: { socketPath, shimSource } })
        │       open() → Host.fs write .demi-bin/<sessionId>/ + PATH
        │       runCommandLine() → env.exec(cd && name args…)
        │
        └─ startCommandBridge(server)        [@demicodes/agent/command-bridge]
                UDS POST /run → runCommandLine
```

```text
Session workspace (LocalHost)
  PATH=.demi-bin/<sessionId>:…
  DEMI_COMMAND_BRIDGE_SOCK=…
  DEMI_SESSION_ID=<sessionId>   # already exported by shell sessions

  node child ──► editor create foo   (symlink → .dispatch → UDS)
```

---

## Package responsibilities

| Package | Role |
|---------|------|
| `@demicodes/shell` | Command tree, exec, abort, quoting (`shellQuote`, `heredocDelimiter`), `MAX_TIMEOUT_MS`. No sockets. |
| `@demicodes/agent` (neutral) | Optional low-level `commandBridge` on `AgentServer`; materialize via `Host.fs`; `runCommandLine`. No Node listener. |
| `@demicodes/agent/command-bridge` (Node-only) | Shim script source + UDS listener. |
| **`@demicodes/host-local`** | `LocalHost` **and** **`createLocalAgentServer`**: default-on bridge assembly (socket path, listener lifecycle, AgentServer options). Depends on `@demicodes/agent`. |
| Products (web, REPL, …) | Prefer `createLocalAgentServer`. Do not re-assemble bridge pieces. |

### Layering note

`host-local` depends on `agent` so local assembly is co-located with `LocalHost`. Platform-neutral `agent` root still must not import `host-local`. This is intentional: **local open-box lives next to LocalHost**, not in every product.

---

## `createLocalAgentServer` contract

```ts
createLocalAgentServer(options: {
  host: LocalHost
  agent: AgentHarness<unknown>
  providers: Provider[]
  shell?: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  session?: AgentServerSessionOptions
  /** Default true. Set false to disable command bridge. */
  commandBridge?: boolean
  /** Optional override; default `<cwd>/.demi/command-bridge.sock`. */
  commandBridgeSocketPath?: string
}): {
  server: AgentServer
  host: LocalHost
  close(): Promise<void>  // stops listener (if any) then server
}
```

When `commandBridge !== false`:

1. Choose `socketPath` (option or `<host.defaultCwd>/.demi/command-bridge.sock`).
2. Construct `AgentServer` with `commandBridge: { socketPath, shimSource: COMMAND_BRIDGE_SHIM_SOURCE }` and shell `initialEnv.PATH` including `process.env.PATH` when missing.
3. `startCommandBridge(server, { socketPath })`.
4. `close()` stops the listener and closes the server (and removes the socket file).

When `commandBridge === false`: plain `AgentServer` only — same as today without bridge.

---

## Runtime pieces (unchanged ideas)

### Shim dir (on each session `open`)

```text
.demi-bin/<agentSessionId>/
  package.json      # {"type":"commonjs"}
  .dispatch         # COMMAND_BRIDGE_SHIM_SOURCE
  <rootName> -> .dispatch
```

PATH prepend realpath of that dir; set `DEMI_COMMAND_BRIDGE_SOCK`.

### Protocol

UDS HTTP/1.1, one connection per call:

```text
POST /run
{ commandScopeId, name, args, cwd, stdin }
→ 200 { exitCode, stdout, stderr }
```

### Stdin (shim)

- No byte within grace window (300ms) → empty stdin.
- Any byte within window → read to EOF without time cap.
- Late data after empty dispatch → stderr diagnostic.

### Exec

`cd <cwd> && <name> <args…>` (+ heredoc if stdin non-empty), `timeoutMs: MAX_TIMEOUT_MS`, cancel via connection `AbortSignal`.

---

## Acceptance (LocalHost)

### Automated

1. `createLocalAgentServer` default: after open, `.demi-bin/<id>/` exists with root symlinks.
2. Real Node child bareword-invokes a registered command successfully.
3. `commandBridge: false`: no `.demi-bin` after open.
4. Stdin through shim reaches the command.
5. Low-level unit tests for materialize / `runCommandLine` / UDS remain.

### Visible (web + LocalHost)

Web uses `createLocalAgentServer` (default on). In UI, exercise a path that spawns a real child which bareword-calls a registered command (or temporary test command); observe shell output and workspace files.

---

## Key decisions

| Decision | Choice |
|----------|--------|
| Open box | `createLocalAgentServer` on `@demicodes/host-local` |
| Default | Bridge **on** |
| Off switch | `commandBridge: false` only |
| Product wiring | Call factory; no socket/shim assembly |
| Agent neutral | Low-level opt-in remains for tests |
| Host package dep | `host-local` → `agent` for assembly |

---

## Migration for products

```ts
// Before
const host = new LocalHost(cwd)
const server = new AgentServer({ agent: harness, providers, shell })

// After
const host = new LocalHost(cwd)
const harness = createCodingAgentHarness({ host })
const { server, close } = createLocalAgentServer({ host, agent: harness, providers, shell })
// on shutdown: await close()
```
