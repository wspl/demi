# Demi

Demi is a hosted coding-agent product that people use in a web browser. A user
talks to an agent in a conversation. The agent runs in the backend and works on
the user's files through a runner, either on a device the user paired or on the
user's Cloud, a gVisor sandbox on a Linux host. The backend, the runner, the
native command programs and the Cloud machine manager are Go programs in one
Go module; the browser application is Vue and TypeScript.

- **Provider-agnostic.** One provider contract, with providers for Claude
  Code, Codex, the Anthropic API, the OpenAI API, Google Gemini and Grok Build.
- **One backend, many targets.** A conversation runs on its user's Cloud or on
  a paired device, through the same runner; switching targets is a first-class
  operation.
- **One command manifest.** Every root command (`demi …`) is declared once in
  the backend and served to every surface; the runner caches the manifest and
  makes the roots real executables.
- **Protocols carry references, never bulk bytes.** Output stays on the target,
  media reaches the browser by reference, and file contents travel as brokered
  HTTP streams.

The design is documented under [docs/](docs/README.md). Start with the
[overview](docs/overview.md);
[Packages](docs/architecture/packages.md) defines what
every Go and TypeScript package owns.

## Development

You need:

- Go. `go.mod` names the toolchain (`toolchain` line), which the `go` command
  downloads when your installation is older. Builds need no C compiler: every
  program builds with `CGO_ENABLED=0`, for every target
  ([Toolchain](docs/delivery/builds-and-releases.md#toolchain)).
- Bun, for the browser packages.
- Node, for the web application's dev server, `bun run web:dev`
  ([Development and checks](docs/product/web-application.md#development-and-checks)).

```sh
go build ./...          # every Go package
go test ./...           # the Go tests
scripts/check.sh ./...  # the full check: six targets, lint, boundaries, race tests

bun install
bun run contracts       # generate the browser's TypeScript contracts from the Go types
bun run typecheck:web   # type-check web-ui, web-gallery and web
bun run test            # the TypeScript tests
bun run web:dev         # the web application, against a running backend
bun run web:gallery     # the component gallery
```

[Validation](docs/delivery/builds-and-releases.md#validation) describes the
checks a change passes.

To run the product locally with the native programs you built, package
development releases of the targets your Hosts use, name the command releases
in a `DEMI_NATIVE_CONFIG` whose store is `local`, and start the backend; it
serves the runners those programs itself:

```sh
go run ./tools/release native build --target <triple>...
go run ./tools/release native package --package demi-runner --output .cache/releases/runners --target <triple>...
go run ./tools/release native package --package demi-file --output .cache/releases/demi-file --target <triple>...
go run ./tools/release native package --package demi-browser --output .cache/releases/demi-browser --target <triple>...
go run ./tools/release native package --package demi-claude-code --output .cache/releases/demi-claude-code --target <triple>...
go build -o .cache/bin/demi-backend ./cmd/demi-backend
DEMI_NATIVE_CONFIG=.cache/releases/native.json DEMI_RUNNER_RELEASE_DIR=.cache/releases/runners \
  DEMI_INSTANCE_MODE=isolated DEMI_BACKEND_PUBLIC_URL=<URL> DEMI_MACHINE_MANAGER_SOCKET=<socket> \
  .cache/bin/demi-backend
```

[Development backend](docs/backend/backend.md#development-backend) gives each
step and variable; for work on the web app alone, `go run ./tools/release dev`
starts a backend with a scripted Cloud and an echo model. The generated TypeScript is not committed; the frontend
scripts that need it generate it first.
[Web application](docs/product/web-application.md#development-and-checks)
describes running the product locally, and
[Builds and releases](docs/delivery/builds-and-releases.md) covers cross builds
and release packages. [CONTRIBUTING.md](CONTRIBUTING.md) lists the rules a
change must follow.

## Extending

- **A new provider.** Add a provider package that implements the provider
  contract for one vendor family; see
  [Add a provider](docs/guides/add-a-provider.md).

## License

[Apache-2.0](LICENSE).
