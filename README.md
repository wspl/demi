# Demi

Demi is a TypeScript toolkit for building agents and coding agents. It gives you a
provider-agnostic agent runtime, a sandboxable shell, a transport-neutral
client/server protocol, and a ready-made coding-agent harness — composable
packages you assemble into your own app.

- **Provider-agnostic** — one inference contract (`@demi/provider`); ship adapters
  for Claude Code, Codex, the Anthropic API, the OpenAI API, or your own.
- **Host-abstracted** — the shell runs against a `Host` (`fs` / `process` / `store`),
  with a Node reference (`@demi/host-local`) and room for remote/container/sandbox backends.
- **Transport-neutral** — drive a session in-process, over stdio, or over WebSocket;
  the same `AgentClient` protocol powers the REPL and the web UI.
- **Long-running shell control** — `shell_exec` / `shell_status` / `shell_write` /
  `shell_abort` / `yield`, with delayed wakeups and budgeted output. See
  [docs/shell-yield-control-plan.md](docs/shell-yield-control-plan.md).

> Status: pre-1.0. The package layout and runtime are stable and tested; the public
> API may still shift before the first published release.

## Architecture

Packages depend strictly downward (enforced by a boundary test):

```
utils, core            shared helpers + data types (zero deps)
provider               abstract inference contract  -> core
shell                  bash engine + Host contract  -> just-bash, utils
host-local             Node Host adapter            -> shell
agent                  session runtime + protocol   -> core, provider, shell, utils
coding-agent           coding harness + commands    -> agent, core, shell, utils
provider-*             concrete providers           -> core, provider, utils
repl / web / web-ui    apps (leaves)
```

The boundary contract is [docs/package-boundaries.md](docs/package-boundaries.md);
runtime/shell design lives under [docs/](docs/).

## Install

```sh
npm install @demi/agent @demi/coding-agent @demi/host-local @demi/provider-claude-code
```

(Packages publish as ESM with bundled `.d.ts`.)

## Quickstart

Assemble a coding agent and drive it through an in-process client:

```ts
import { AgentServer } from '@demi/agent'
import { createCodingAgentHarness } from '@demi/coding-agent'
import { LocalHost } from '@demi/host-local'
import { createClaudeCodeProvider, listClaudeCodeModels } from '@demi/provider-claude-code'

// 1. A Host gives the agent a filesystem, process spawning, and a scoped store.
const host = new LocalHost(process.cwd())

// 2. A harness supplies the system prompt, registered commands, and reference resolution.
const harness = createCodingAgentHarness({ host })

// 3. One or more inference providers.
const providers = [createClaudeCodeProvider()]

// 4. The server owns the session lifecycle; get an in-process client.
const server = new AgentServer({
  agent: harness,
  providers,
  shell: { initialEnv: { PATH: process.env.PATH ?? '' } },
})
const client = server.client()

// 5. Render transcript updates (the same blocks the REPL and web UI render).
client.subscribe((event) => {
  if (event.type === 'transcript_snapshot' || event.type === 'transcript_patch') {
    // event.blocks -> your UI
  }
})

// 6. Open a session with a model from the provider's catalog, then send a turn.
const [model] = await listClaudeCodeModels()
await client.open({ providerId: 'claude-code', model: { providerId: 'claude-code', model, thinking: null } }, process.cwd())
await client.send([{ type: 'text', text: 'Create hello.txt with "hi", then read it back.' }])
```

For a terminal UI over the same protocol, see `@demi/repl`; for a browser UI that
consumes an injected `AgentClient`, see `@demi/web-ui`.

## Extending

- **A new provider** — implement the `@demi/provider` contract (`run()` returning a
  `ProviderRun` of `ProviderEvent`s) and export a `createXProvider()` factory.
- **A new Host** — implement `{ defaultCwd, fs, process, store }`; `@demi/host-local`
  is the reference for a remote/container/sandboxed backend.
- **A new UI** — consume an `AgentClient` (in-process, stdio via `@demi/agent/stdio`,
  or WebSocket) and render `Block`s per [docs/tool-rendering-spec.md](docs/tool-rendering-spec.md).

## Development

```sh
bun install
bun run typecheck      # type-check all packages
bun run typecheck:web  # type-check the Vue UI packages
bun run test           # run the test suite
bun run build          # build every library package to dist/ (tsdown)
```

Workspaces resolve `@demi/*` from source in dev/test (the `development` export
condition); a build is only needed to publish.

## License

[Apache-2.0](LICENSE). Demi bundles a fork of `just-bash` (also Apache-2.0) as the
bash engine behind `@demi/shell`; see [NOTICE](NOTICE).
