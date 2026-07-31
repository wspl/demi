# @demicodes/host-local

The Node reference implementation of the `@demicodes/shell` `Host` contract: real
filesystem, process spawning, and a durable JSON store under the platform data
directory (`…/demi/host-local/…`). Also owns open-box local agent assembly —
command bridge **on by default**.

```ts
import { LocalHost, createLocalAgentServer } from '@demicodes/host-local'

const host = new LocalHost(process.cwd())

// Prefer createLocalAgentServer for local products: it wires the command-bridge
// UDS listener and PATH shims instead of hand-assembling AgentServer.
const server = createLocalAgentServer({ agent: harness, providers })
```

- `LocalHost` — `Host` with full local `fs` / `process` / `store`.
- `createLocalAgentServer` — local `AgentServer` factory with command bridge
  defaults (see [docs/command-bridge.md](../../docs/command-bridge.md)).
- Command-bridge state under `~/.demi` / `$DEMI_HOME`: `bridges/`, `bridge-bin/`.

> `LocalHost` grants full local filesystem and process access. For untrusted
> agents, supply a sandboxing `Host` instead — see
> [Implement a Host](../../docs/guides/implement-a-host.md).

Part of [Demi](../../README.md). Apache-2.0.
