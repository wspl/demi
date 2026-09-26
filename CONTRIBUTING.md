# Contributing to Demi

Thanks for your interest in Demi. This guide covers local setup, the checks a
change must pass, the architecture rules those checks enforce, and how to
extend Demi.

## Prerequisites

- Rust through [rustup](https://rustup.rs). `rust-toolchain.toml` pins the
  toolchain and lists its targets, which rustup installs with it. Host builds
  also need a C compiler.
- The cross tools, to build for a platform other than your own
  ([Toolchain](docs/delivery/builds-and-releases.md#toolchain)).
- [Bun](https://bun.sh), for the browser packages.

## Setup and checks

```sh
cargo check --workspace --all-targets --features demi-runner/test-fixtures
cargo test --workspace --features demi-runner/test-fixtures  # the Rust tests and the crate boundary check

bun install
bun run typecheck:web   # type-check web-ui, web-gallery and web
bun run test            # the TypeScript tests and the package boundary check
```

Every Rust command selects the whole workspace with the runner's test
fixtures, so they share one build; [Validation](docs/delivery/builds-and-releases.md#validation)
lists the Chrome suite and how a test finds the programs it starts.

Rust types are the only definition of every wire and stored format
([Contracts](docs/architecture/contracts.md)). The browser's TypeScript types
and Zod schemas are generated from them and are not committed: the frontend
scripts generate them before they run, and `bun run contracts` regenerates
them on its own. After you change a type the browser uses,
`bun run typecheck:web` shows every place in the frontend that the change
breaks.

## Architecture rules

1. **Crates and packages.**
   [Crates and packages](docs/architecture/crates-and-packages.md) is the
   highest architectural constraint: what each crate and package owns, its
   public boundary, and what it must not do. Its two dependency graphs are
   read by the boundary checks, so the Rust tests fail when a crate's
   dependencies differ from the Rust graph, and `bun run test` fails when a
   browser package's differ from the TypeScript graph. A new crate, package or
   dependency between them starts with a change to that document.
2. **One owner per helper.** Before you write a helper, search the standard
   library, the crate's declared dependencies and the workspace, in that order;
   in the browser packages, search the package's declared dependencies,
   `@demicodes/utils` and the workspace. Two implementations of the same
   purpose are a defect: import the existing one and merge duplicates.
   Domain-specific helpers stay in the crate or package that owns the domain.
3. **Concurrency.** State has one owner, read-mostly data is published as a
   snapshot, and blocking work leaves async threads
   ([Concurrency](docs/architecture/concurrency.md)).
4. **Validation at entry.** A value from outside the process is decoded into
   its type and validated where it enters. Nothing is asserted onto a value,
   and corrupt data is refused, never repaired
   ([Validation at entry](docs/architecture/contracts.md#validation-at-entry)).

[AGENTS.md](AGENTS.md) holds the full working principles, writing rules and
coding standards.

## Tests

- Rust unit tests sit beside the code, and each crate has one integration test
  binary ([Module layout](docs/architecture/crates-and-packages.md#module-layout)).
  [Tests and time](docs/architecture/concurrency.md#tests-and-time) covers
  clocks, real processes and built binaries.
- No automated test calls a real model. Tests use scripted providers and
  fixtures, so a run costs nothing and answers the same way every time.
- Suites that need a real machine manager, Chrome or the Claude Code CLI run
  only when environment variables supply those resources
  ([Real machine acceptance](docs/delivery/scenarios.md#real-machine-acceptance)).
- Keep the suite green: before every commit, run the Rust tests, and when a
  browser package changed, `bun run typecheck:web` and `bun run test`.

## Commits

- Use [Conventional Commits](https://www.conventionalcommits.org/) subjects
  (`feat:`, `fix:`, `refactor:`, `docs:`, `test:`…).
- Keep changes scoped; commit working checkpoints.

## Extending Demi

- **A new provider.** Add a provider crate that implements the provider
  contract for one vendor family; see
  [Add a provider](docs/guides/add-a-provider.md).

## License

By contributing, you agree that your contributions are licensed under the
project's [Apache-2.0](LICENSE) license.
