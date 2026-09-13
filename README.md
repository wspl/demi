# Demi

Demi is a hosted coding-agent product and the TypeScript packages it is
built from: a provider-agnostic agent runtime, a command system with one
manifest for every execution surface, and a runner that
turns any machine into an execution target. The design is recorded under
[docs/demi-next/](docs/demi-next/overview.md); the package contract is
[docs/package-boundaries.md](docs/package-boundaries.md).

- **Provider-agnostic** — one inference contract (`@demicodes/provider`) with
  adapters for Claude Code, Codex, the Anthropic API, the OpenAI API, Google
  Gemini and Grok Build.
- **One backend, many targets** — a conversation runs on its user’s managed
  Cloud or a connected device, through the same runner; switching targets is a first-class operation.
- **One command manifest** — every root command (`demi …`) is defined once in
  the backend and served to every surface; the runner caches it and makes the
  roots real executables.
- **Protocols carry references, never bulk bytes** — output stays on the
  target, media reaches the browser by reference, transfers are brokered HTTP
  streams.

> Status: pre-1.0, delivered milestone by milestone
> ([docs/demi-next/roadmap.md](docs/demi-next/roadmap.md)).

## Architecture

Package responsibilities and allowed dependencies are defined in
[package boundaries](docs/package-boundaries.md) and enforced by boundary tests.
The backend and agent SDK run TypeScript. Execution targets run a native Rust
runner with an embedded shell and standard utilities. Agent commands select
application callbacks or independently distributed resident native services.
See [native execution](docs/demi-next/native-runtime.md).

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

Install Rust through rustup (the repository pins its toolchain), a C/C++ compiler
and CMake. Tests build the native runner automatically and use scripted providers.
`cargo test --workspace` runs the Rust protocol, service and runner tests.
Six-target release commands and SDK requirements are in
[native builds](docs/native-builds.md).

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
