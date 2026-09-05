# Demi

Demi is a hosted coding-agent product and the TypeScript packages it is
built from: a provider-agnostic agent runtime, a command system with one
manifest for every execution surface, a hostless shell, and a runner that
turns any machine into an execution target. The design is recorded under
[docs/demi-next/](docs/demi-next/overview.md); the package contract is
[docs/package-boundaries.md](docs/package-boundaries.md).

- **Provider-agnostic** — one inference contract (`@demicodes/provider`) with
  adapters for Claude Code, Codex, the Anthropic API, the OpenAI API, Google
  Gemini and Grok Build.
- **One backend, many targets** — a conversation runs hostless (files in its
  own store, scripts in `@demicodes/tinybash`) or on a machine reached through
  its runner; switching targets is a first-class operation.
- **One command manifest** — every root command (`demi …`) is defined once in
  the backend and served to every surface; the runner caches it and makes the
  roots real executables.
- **Protocols carry references, never bulk bytes** — output stays on the
  target, media reaches the browser by reference, transfers are brokered HTTP
  streams.

> Status: pre-1.0, delivered milestone by milestone
> ([docs/demi-next/roadmap.md](docs/demi-next/roadmap.md)).

## Architecture

Packages depend strictly downward (enforced by a boundary test):

```
utils, core            shared helpers + data types (zero deps)
provider               abstract inference contract          -> core, utils
tinybash               the hostless shell (standalone)      -> utils
shell                  Host contract + command system       -> tinybash (hostless entry), utils
agent                  session runtime + protocol           -> core, provider, shell, utils
coding-agent           coding harness + the demi root       -> agent, core, shell, utils
provider-*             concrete providers                   -> core, provider, utils
host-virtual           the hostless Host                    -> shell, utils
command-loader         manifest + loader                    -> shell, utils
runner-protocol        the runner wire                      -> shell, utils
host-remote            the backend's Host over a runner     -> runner-protocol, shell, utils
runner                 the machine-side program (tinyjs)    -> command-loader, runner-protocol, shell, utils
backend                the product server (leaf)
web-ui, web-gallery    the browser UI library and component gallery
```

Notable design records outside `docs/demi-next/`:

- [Provider quota](docs/provider-quota.md) — unified probe/observe for subscription rate limits
- [Provider global credentials](docs/provider-global-credentials.md) — multi-account pool + global `setActive`
- [Provider / session clone](docs/provider-session-clone.md) — required `.clone()` for isolated forks
- [Subagents](docs/subagent.md) — child sessions as `demi agent`, subagent events on the parent `AgentClient`
- [Provider errors & retries](docs/provider-errors-and-retries.md) — classified failures and resume recovery

## Development

```sh
bun install
bun run typecheck      # type-check all packages
bun run typecheck:web  # type-check the Vue UI packages
bun run test           # run the test suite
bun run build          # build every library package to dist/ (tsdown)
bun run llms           # regenerate llms-full.txt from the docs
```

The tinyjs runtime is a Rust crate under `packages/tinyjs`
([docs/demi-next/tinyjs.md](docs/demi-next/tinyjs.md)); tests that need it
build it once through `@demicodes/runner/testing`.

Workspaces resolve `@demicodes/*` from source in dev/test (the `development` export
condition); a build is only needed to publish.

## Extending

- **A new provider** — implement the `@demicodes/provider` contract (`run()` returning a
  `ProviderRun` of `ProviderEvent`s) and export a `createXProvider()` factory.
  See [docs/guides/add-a-provider.md](docs/guides/add-a-provider.md).
- **A new UI** — consume an `AgentClient` and render `Block`s per
  [docs/tool-rendering-spec.md](docs/tool-rendering-spec.md).

## License

[Apache-2.0](LICENSE).
