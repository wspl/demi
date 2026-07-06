# Demi

Demi is a TypeScript toolkit for building agents and coding agents. It gives you a
provider-agnostic agent runtime, a sandboxable shell, a transport-neutral
client/server protocol, and a ready-made coding-agent harness — composable
packages you assemble into your own app.

- **Provider-agnostic** — one inference contract (`@demicodes/provider`); ship adapters
  for Claude Code, Codex, the Anthropic API, the OpenAI API, or your own.
- **Host-abstracted** — the shell runs against a `Host` (`fs` / `process` / `store`),
  with a Node reference (`@demicodes/host-local`) and room for remote/container/sandbox backends.
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
host-local             Node Host adapter            -> shell, utils
agent                  session runtime + protocol   -> core, provider, shell, utils
coding-agent           coding harness + commands    -> agent, core, shell, utils
provider-*             concrete providers           -> core, provider, utils
repl / web / web-ui    apps (leaves)
```

The boundary contract is [docs/package-boundaries.md](docs/package-boundaries.md);
runtime/shell design lives under [docs/](docs/).

## Install

```sh
npm install @demicodes/agent @demicodes/coding-agent @demicodes/host-local @demicodes/provider-claude-code
```

(Packages publish as ESM with bundled `.d.ts`.)

## Quickstart

Assemble a coding agent and drive it through an in-process client:

```ts
import { AgentServer } from '@demicodes/agent'
import { createCodingAgentHarness } from '@demicodes/coding-agent'
import { LocalHost } from '@demicodes/host-local'
import { modelSelectionFromCatalog } from '@demicodes/provider'
import { createClaudeCodeProvider, listClaudeCodeModels } from '@demicodes/provider-claude-code'

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
const catalog = await listClaudeCodeModels()
const model = catalog.models.find((m) => m.id === catalog.defaultModelId) ?? catalog.models[0] ?? null
await client.open({ providerId: 'claude-code', model: modelSelectionFromCatalog('claude-code', model) }, process.cwd())
await client.send([{ type: 'text', text: 'Create hello.txt with "hi", then read it back.' }])
```

For a terminal UI over the same protocol, see `@demicodes/repl`; for a browser UI that
consumes an injected `AgentClient`, see `@demicodes/web-ui`.

## Extending

- **A new provider** — implement the `@demicodes/provider` contract (`run()` returning a
  `ProviderRun` of `ProviderEvent`s) and export a `createXProvider()` factory.
  See [docs/guides/add-a-provider.md](docs/guides/add-a-provider.md) and the runnable
  template [examples/custom-provider.ts](examples/custom-provider.ts).
- **A new Host** — implement `{ defaultCwd, fs, process, store }`; `@demicodes/host-local`
  is the reference for a remote/container/sandboxed backend.
  See [docs/guides/implement-a-host.md](docs/guides/implement-a-host.md) and the
  sandbox template [examples/sandboxed-host.ts](examples/sandboxed-host.ts).
- **A new UI** — consume an `AgentClient` (in-process, stdio via `@demicodes/agent/stdio`,
  or WebSocket) and render `Block`s per [docs/tool-rendering-spec.md](docs/tool-rendering-spec.md).
  See [docs/guides/embed-the-ui.md](docs/guides/embed-the-ui.md).

## Development

```sh
bun install
bun run typecheck      # type-check all packages
bun run typecheck:web  # type-check the Vue UI packages
bun run test           # run the test suite
bun run build          # build every library package to dist/ (tsdown)
bun run llms           # regenerate llms-full.txt from the docs
```

Workspaces resolve `@demicodes/*` from source in dev/test (the `development` export
condition); a build is only needed to publish.

## License

[Apache-2.0](LICENSE). Demi bundles a fork of `just-bash` (also Apache-2.0) as the
bash engine behind `@demicodes/shell`; see [NOTICE](NOTICE).
