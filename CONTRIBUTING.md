# Contributing to Demi

Thanks for your interest in Demi. This guide covers local setup, the checks a
change must pass, the architecture rules those checks enforce, and how to
extend Demi.

## Issues and pull requests

Demi doesn't accept public pull requests. To report a bug or ask for a
feature, open an [issue](https://github.com/wspl/demi/issues/new/choose). To
become a contributor, email [join@demi.sh](mailto:join@demi.sh).

## Prerequisites

- Rust through [rustup](https://rustup.rs). `rust-toolchain.toml` pins the
  toolchain and lists its targets, which rustup installs with it. Host builds
  also need a C compiler.
- The cross tools, to build for a platform other than your own
  (`crates/xtask/src/native/mod.rs`).
- [Bun](https://bun.sh), for the browser packages.
- [Node](https://nodejs.org), for the web app's development server, which
  runs Vite under Node.

`AGENTS.md` holds the rules the coding agents working in this repository
follow, and `.demi/workflows/` the maintainers' subagent workflow for Demi
itself; neither is a requirement for a human contribution, which this guide
covers.

## Setup and checks

```sh
cargo check --workspace --all-targets --features demi-runner/test-fixtures
cargo test --workspace --features demi-runner/test-fixtures  # the Rust tests and the crate boundary check

bun install
bun run typecheck:web   # type-check web-ui, web-gallery and web
bun run test            # the TypeScript tests and the package boundary check
```

Every Rust command selects the whole workspace with the runner's test
fixtures, so they share one build; `crates/xtask/src/main.rs` lists the
Chrome suite and how a test finds the programs it starts.

Rust types are the only definition of every wire and stored format
([Contracts](ARCHITECTURE.md#contracts)). The browser's TypeScript types
and Zod schemas are generated from them and are not committed: the frontend
scripts generate them before they run, and `bun run contracts` regenerates
them on its own. After you change a type the browser uses,
`bun run typecheck:web` shows every place in the frontend that the change
breaks.

## Run Demi locally

```sh
bun install
bun xtask dev         # a development backend at :3271, with a scripted Cloud and an account
bun run web:dev       # in another terminal: the web app at http://127.0.0.1:18934
bun run web:gallery   # the component gallery, with no backend
```

`bun xtask dev` builds the backend, starts it on a fresh data directory with
a scripted Cloud and an Echo model, makes an account, and prints the account
and the exact `web:dev` command to run; `.env` (copy `.env.example`) can name
the account and add a real model. `--unseeded` makes no account, so the page
opens on Set Up Demi, and `--data <dir>` keeps the data between runs
(`crates/xtask/src/dev/mod.rs`).

A real Cloud needs Linux with gVisor. To run a server release built from your
checkout, with the native programs it serves to runners, follow
[A development build on a server](docs/delivery/upgrades.md#a-development-build-on-a-server).

## The README

`README.md` and its translations, `README.zh-CN.md`, `README.zh-TW.md`,
`README.ja.md` and `README.ko.md`, say the same thing section for section. A
change to one changes all of them in the same commit; the English one is the
reference when they differ. The comparison, `docs/comparison.md`, and its translations
follow the same rule.

## Architecture rules

1. **Crates and packages.** [ARCHITECTURE.md](ARCHITECTURE.md) is the map
   of the code and the rules across it; what each crate and package owns and
   must not do is in its entry comment (`lib.rs`, `main.rs`, `index.ts`). Its
   two dependency graphs are read by the boundary checks, so the Rust tests
   fail when a crate's dependencies differ from the Rust graph, and
   `bun run test` fails when a browser package's differ from the TypeScript
   graph. A new crate, package or dependency between them starts with a
   change to its graph line.
2. **One owner per helper.** Before you write a helper, search the standard
   library, the crate's declared dependencies and the workspace, in that order;
   in the browser packages, search the package's declared dependencies,
   `@demicodes/utils` and the workspace. Two implementations of the same
   purpose are a defect: import the existing one and merge duplicates.
   Domain-specific helpers stay in the crate or package that owns the domain.
3. **Concurrency.** State has one owner, read-mostly data is published as a
   snapshot, and blocking work leaves async threads
   ([Concurrency](ARCHITECTURE.md#concurrency)).
4. **Validation at entry.** A value from outside the process is decoded into
   its type and validated where it enters. Nothing is asserted onto a value,
   and corrupt data is refused, never repaired
   (`crates/shared-types/src/lib.rs`).

[AGENTS.md](AGENTS.md) holds the full working principles, writing rules and
coding standards.

## Tests

[Testing](docs/delivery/testing.md) says what a test protects, where it runs,
how it proves itself, what it may cost and how coverage is used; read it
before you add or change a test.

- Rust unit tests sit beside the code, and each crate has one integration test
  binary ([Test binaries](ARCHITECTURE.md#test-binaries)).
  [Tests and time](ARCHITECTURE.md#tests-and-time) covers
  clocks, real processes and built binaries.
- Keep the suite green: before every commit, run the Rust tests, and when a
  browser package or JavaScript that a crate ships changed,
  `bun run typecheck:web` and `bun run test`.

## Commits

- Use [Conventional Commits](https://www.conventionalcommits.org/) subjects
  (`feat:`, `fix:`, `refactor:`, `docs:`, `test:`…).
- Keep changes scoped; commit working checkpoints.

## Extending Demi

- **A new provider.** Add a provider crate that implements the provider
  contract for one vendor family; see
  [Add a provider](docs/guides/add-a-provider.md).

## License

Contributions are licensed under the license of the part they change. Most of
Demi is under the [Apache License 2.0](LICENSE), and a contribution there is
licensed under it, as its section 5 says. A contribution to the server or the
web app, the parts [LICENSE-DEMI](LICENSE-DEMI) lists, is licensed under the
Demi License, and you agree that The Demi Authors may also license it to
others under other terms, including terms that permit what that license
otherwise forbids.
