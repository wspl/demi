# Contributing to Demi

Thanks for your interest in Demi. This guide covers local setup, the checks a
change must pass, the architecture rules those checks enforce, and how to
extend Demi.

## Prerequisites

- [Go](https://go.dev/dl/). `go.mod` names the toolchain (`toolchain` line),
  which the `go` command downloads when your installation is older. Builds
  need no C compiler: every program builds with `CGO_ENABLED=0`, for every
  target ([Toolchain](docs/delivery/builds-and-releases.md#toolchain)).
- [Bun](https://bun.sh), for the browser packages.

## Setup and checks

```sh
go build ./...          # every Go package
go test ./...           # the Go tests
scripts/check.sh ./...  # the full check: six targets, lint, the package boundary check, race tests

bun install
bun run typecheck:web   # type-check web-ui, web-gallery and web
bun run test            # the TypeScript tests and the package boundary check
```

[Validation](docs/delivery/builds-and-releases.md#validation) lists the
Chrome suite and how a test finds the programs it starts.

Go types are the only definition of every wire and stored format
([Contracts](docs/architecture/contracts.md)). The browser's TypeScript types
and Zod schemas are generated from them and are not committed: the frontend
scripts generate them before they run, and `bun run contracts` regenerates
them on its own. After you change a type the browser uses,
`bun run typecheck:web` shows every place in the frontend that the change
breaks.

## Architecture rules

1. **Packages.**
   [Packages](docs/architecture/packages.md) is the
   highest architectural constraint: what each package owns, its public
   boundary, and what it must not do. Its two dependency graphs are read by
   the boundary checks, so `scripts/check.sh` fails when a Go package's
   imports differ from the Go graph, and `bun run test` fails when a browser
   package's differ from the TypeScript graph. A new package or dependency
   between packages starts with a change to that document.
2. **One owner per helper.** Before you write a helper, search the standard
   library, the module's declared dependencies and the repository, in that order;
   in the browser packages, search the package's declared dependencies,
   `@demicodes/utils` and the workspace. Two implementations of the same
   purpose are a defect: import the existing one and merge duplicates.
   Domain-specific helpers stay in the package that owns the domain.
3. **Concurrency.** State has one owner, read-mostly data is published as a
   snapshot, every goroutine has an owner that cancels it and waits for it,
   and every lease is released with `defer` where it is acquired
   ([Concurrency](docs/architecture/concurrency.md)).
4. **Validation at entry.** A value from outside the process is decoded into
   its type and validated where it enters. Nothing is asserted onto a value,
   and corrupt data is refused, never repaired
   ([Validation at entry](docs/architecture/contracts.md#validation-at-entry)).

[AGENTS.md](AGENTS.md) holds the full working principles, writing rules and
coding standards.

## Tests

[Testing](docs/delivery/testing.md) says what a test protects, where it runs,
how it proves itself, what it may cost and how coverage is used; read it
before you add or change a test.

- Go tests sit beside the code they test, in `_test.go` files of its package
  ([Module layout](docs/architecture/packages.md#module-layout)).
  [Tests and time](docs/architecture/concurrency.md#tests-and-time) covers
  clocks, real processes and built binaries.
- Keep the suite green: before every commit, run the Go tests, and when a
  browser package or JavaScript that a Go program embeds changed,
  `bun run typecheck:web` and `bun run test`.

## Commits

- Use [Conventional Commits](https://www.conventionalcommits.org/) subjects
  (`feat:`, `fix:`, `refactor:`, `docs:`, `test:`…).
- Keep changes scoped; commit working checkpoints.

## Extending Demi

- **A new provider.** Add a provider package that implements the provider
  contract for one vendor family; see
  [Add a provider](docs/guides/add-a-provider.md).

## License

By contributing, you agree that your contributions are licensed under the
project's [Apache-2.0](LICENSE) license.
