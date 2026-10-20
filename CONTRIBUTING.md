# Contributing to Demi

Thanks for your interest in Demi. This guide covers:

- Local setup
- The checks a change must pass
- The architecture rules those checks enforce
- How to extend Demi

## Issues and pull requests

Demi doesn't accept public pull requests. To report a bug or ask for a
feature, open an [issue](https://github.com/wspl/demi/issues/new/choose). To
become a contributor, email [join@demi.sh](mailto:join@demi.sh).

## Prerequisites

You need the following:

- Rust through [rustup](https://rustup.rs). `rust-toolchain.toml` pins the
  toolchain and lists its targets, which rustup installs with it. Host builds
  also need a C compiler.
- The cross tools, to build for a platform other than your own
  (`crates/xtask/src/native/mod.rs`).
- [Bun](https://bun.sh), for the browser packages.
- [Node](https://nodejs.org), for the web app's development server, which
  runs Vite under Node.

Two other places hold rules for agents, and neither is a requirement for a
human contribution, which this guide covers:

- `AGENTS.md` holds the rules the coding agents working in this repository
  follow.
- `.demi/workflows/` holds the maintainers' subagent workflow for Demi itself.

## Setup and checks

Set up the repository and check a change with these commands:

```sh
cargo check --workspace --all-targets --features demi-runner/test-fixtures
cargo test --workspace --features demi-runner/test-fixtures  # the Rust tests and the crate boundary check

bun install
bun run typecheck:web   # type-check web-ui, web-gallery and web
bun run test            # the TypeScript tests and the package boundary check
```

Every Rust command selects the whole workspace with the runner's test
fixtures, so they share one build. `crates/xtask/src/main.rs` lists the Chrome
suite and how a test finds the programs it starts.

Rust types are the only definition of every wire and stored format
([Contracts](ARCHITECTURE.md#contracts)). The browser's TypeScript types and
Zod schemas derive from them, and you don't commit them:

- The frontend scripts generate them before they run.
- `bun run contracts` regenerates them on its own.

After you change a type the browser uses, `bun run typecheck:web` shows every
place in the frontend that the change breaks.

## Run Demi locally

Start Demi on your machine with these commands:

```sh
bun install
bun xtask dev         # a development backend at :3271, with a scripted Cloud and an account
bun run web:dev       # in another terminal: the web app at http://127.0.0.1:18934
bun run web:gallery   # the component gallery, with no backend
```

`bun xtask dev` does these steps:

1. Builds the backend.
2. Starts it on a fresh data directory with a scripted Cloud and an Echo model.
3. Makes an account.
4. Prints the account and the exact `web:dev` command to run.

`.env` (copy `.env.example`) can name the account and add a real model. Two
options change the steps (`crates/xtask/src/dev/mod.rs`):

- `--unseeded` makes no account, so the page opens on Set Up Demi.
- `--data <dir>` keeps the data between runs.

A real Cloud needs Linux with gVisor. To run a server release built from your
checkout, with the native programs it serves to runners, follow
[A development build on a server](docs/delivery/upgrades.md#a-development-build-on-a-server).

## The README

`README.md` and its translations, `README.zh-CN.md`, `README.zh-TW.md`,
`README.ja.md` and `README.ko.md`, say the same thing section for section. Two
rules keep them in step:

- A change to one changes all of them in the same commit.
- The English one is the reference when they differ.

The comparison, `docs/comparison.md`, and its translations follow the same
rule.

## Architecture rules

Four rules apply across the code:

- **Crates and packages.** [ARCHITECTURE.md](ARCHITECTURE.md) is the map of
  the code and the rules across it. What each crate and package owns and must
  not do is in its entry comment (`lib.rs`, `main.rs`, `index.ts`).

  The boundary checks read its two dependency graphs:

  - The Rust tests fail when a crate's dependencies differ from the Rust
    graph.
  - `bun run test` fails when a browser package's dependencies differ from the
    TypeScript graph.

  A new crate, package or dependency between them starts with a change to its
  graph line.
- **One owner per helper.** Before you write a helper, search these places in
  order:

  - In Rust: the standard library, the crate's declared dependencies and the
    workspace.
  - In the browser packages: the package's declared dependencies,
    `@demicodes/utils` and the workspace.

  Two implementations of the same purpose are a defect. Import the existing
  one and merge duplicates. Domain-specific helpers stay in the crate or
  package that owns the domain.
- **Concurrency.** Give state one owner, publish read-mostly data as a
  snapshot, and keep blocking work off async threads
  ([Concurrency](ARCHITECTURE.md#concurrency)).
- **Validation at entry.** Decode a value from outside the process into its
  type and validate it where it enters. Assert nothing onto a value, and
  refuse corrupt data instead of repairing it
  (`crates/shared-types/src/lib.rs`).

[AGENTS.md](AGENTS.md) holds the full working principles, writing rules and
coding standards.

## Tests

[Testing](docs/delivery/testing.md) describes what a test protects, where it
runs, how it proves itself, what it can cost and how coverage is used. Read it
before you add or change a test.

Two rules apply to the suite:

- Rust unit tests sit beside the code, and each crate has one integration test
  binary ([Test binaries](ARCHITECTURE.md#test-binaries)).
  [Tests and time](ARCHITECTURE.md#tests-and-time) covers clocks, real
  processes and built binaries.
- Keep the suite green. Before every commit, run the Rust tests. When a
  browser package or JavaScript that a crate ships changed, also run
  `bun run typecheck:web` and `bun run test`.

## Commits

Follow these rules for commits:

- Use [Conventional Commits](https://www.conventionalcommits.org/) subjects
  (`feat:`, `fix:`, `refactor:`, `docs:`, `test:` and so on).
- Keep changes scoped.
- Commit working checkpoints.

## Extending Demi

To add a new provider, add a provider crate that implements the provider
contract for one vendor family. For the steps, see
[Add a provider](docs/guides/add-a-provider.md).

## License

A contribution carries the license of the part it changes:

- Most of Demi is under the [Apache License 2.0](LICENSE). A contribution there
  falls under it, as its section 5 says.
- A contribution to the server or the web app, the parts
  [LICENSE-DEMI](LICENSE-DEMI) lists, falls under the Demi License. You
  agree that The Demi Authors may also license it to others under other terms,
  including terms that permit what that license otherwise forbids.
