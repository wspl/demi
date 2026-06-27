# Contributing to Demi

Thanks for your interest in Demi. This guide covers local setup, the project's
hard architectural rules, and how to extend it.

## Prerequisites

- [Bun](https://bun.sh) (the repo uses Bun workspaces and `bun test`).
- The `just-bash` engine is a git submodule. Clone with submodules:

  ```sh
  git clone --recurse-submodules <repo>
  # or, in an existing checkout:
  git submodule update --init --recursive
  ```

## Setup

```sh
bun install
bun run typecheck      # type-check all packages (tsgo)
bun run typecheck:web  # type-check the Vue UI packages
bun run test           # run the suite (uses --conditions development -> src)
bun run build          # build every library package to dist/ (tsdown)
```

Workspaces resolve `@demicodes/*` from source in dev/test via the `development` export
condition, so no build step is needed to run tests. A build is only required to
publish.

## Architecture rules (please read before opening a PR)

These are enforced by `packages/core/src/__tests__/platform-entrypoints.test.ts`,
so violations fail CI.

1. **Package boundaries.** `docs/package-boundaries.md` is the highest
   architectural constraint. Dependencies point strictly downward and the graph
   stays acyclic. Adding a package means updating that doc *and* the maps in the
   boundary test.
2. **No Node leakage in platform-neutral packages.** `core`, `provider`, `utils`,
   `agent`, `coding-agent`, `shell` and the provider adapters must not import
   Node built-ins from their public entrypoints.
3. **Code reuse is mandatory.** Generic helpers live in `@demicodes/utils` (helpers
   that return a `core` type, like `zeroUsage`, live in `@demicodes/core`). Do **not**
   re-implement, copy-paste, or create a same-purpose-but-differently-named
   helper — import the existing one and merge duplicates. The boundary test bans
   re-defining the consolidated helpers. Domain-specific helpers stay in their
   owning package.

## Tests

- Co-locate tests in `__tests__/` next to the code.
- Real-provider / real-CLI tests are gated behind env vars and end in `.e2e.test.ts`;
  they are not part of the default run.
- Keep the suite green: `bun run typecheck && bun run test` before every commit.

## Commits

- Use [Conventional Commits](https://www.conventionalcommits.org/) subjects
  (`feat:`, `fix:`, `refactor:`, `docs:`, `test:`…).
- Keep changes scoped; commit working checkpoints.

## Extending Demi

- **A new provider** — implement the `@demicodes/provider` contract (a `run()` that
  yields `ProviderEvent`s) and export a `createXProvider()` factory. Reuse the
  shared HTTP helpers (`redactSecretText`, `httpErrorCode`, `normalizeErrorCode`,
  `providerErrorFromUnknown`) and `modelSelectionFromCatalog` rather than
  re-deriving them. See `packages/provider-anthropic-api` as a reference.
- **A new Host** — implement `{ defaultCwd, fs, process, store }`;
  `@demicodes/host-local` is the Node reference for a remote/container/sandbox backend.
- **A new UI** — consume an `AgentClient` (in-process, stdio via `@demicodes/agent/stdio`,
  or WebSocket) and render `Block`s per `docs/tool-rendering-spec.md`.

## License

By contributing, you agree that your contributions are licensed under the
project's [Apache-2.0](LICENSE) license.
