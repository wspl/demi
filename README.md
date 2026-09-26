# Demi

Demi is a hosted coding-agent product that people use in a web browser. A user
talks to an agent in a conversation. The agent runs in the backend and works on
the user's files through a runner, either on a device the user paired or on the
user's Cloud, a gVisor sandbox on a Linux host. The backend, the runner, the
native command programs and the Cloud machine manager are Rust programs in one
Cargo workspace; the browser application is Vue and TypeScript.

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
[Crates and packages](docs/architecture/crates-and-packages.md) defines what
every crate and package owns.

## Development

You need:

- Rust through rustup. `rust-toolchain.toml` pins the toolchain and lists its
  targets, which rustup installs with it. Host builds also need a C compiler.
- The cross tools, to build for a platform other than your own
  ([Toolchain](docs/delivery/builds-and-releases.md#toolchain)).
- Bun, for the browser packages.

```sh
cargo check --workspace --all-targets --features demi-runner/test-fixtures
cargo test --workspace --features demi-runner/test-fixtures  # the Rust tests

bun install
bun run contracts       # generate the browser's TypeScript contracts from the Rust types
bun run typecheck:web   # type-check web-ui, web-gallery and web
bun run test            # the TypeScript tests
bun run web:dev         # the web application, against a running backend
bun run web:gallery     # the component gallery
```

Every Rust command selects the whole workspace with the runner's test
fixtures, so they share one build
([Validation](docs/delivery/builds-and-releases.md#validation)).

To run the product locally with the native programs you built, package
development releases of the targets your Hosts use, name the command releases
in a `DEMI_NATIVE_CONFIG` whose store is `local`, and start the backend; it
serves the runners those programs itself:

```sh
cargo xtask native build --target <triple>...
cargo xtask native package --package demi-runner --output .cache/releases/runners --target <triple>...
cargo xtask native package --package demi-commands --output .cache/releases/demi-builtin --target <triple>...
cargo xtask native package --package demi-claude --output .cache/releases/demi-claude --target <triple>...
cargo build --workspace --all-targets --features demi-runner/test-fixtures
DEMI_NATIVE_CONFIG=.cache/releases/native.json DEMI_RUNNER_RELEASE_DIR=.cache/releases/runners \
  DEMI_INSTANCE_MODE=isolated DEMI_BACKEND_PUBLIC_URL=<URL> DEMI_MACHINES_SOCKET=<socket> \
  target/debug/demi-backend
```

[Development backend](docs/backend/backend.md#development-backend) gives each
step and variable. The generated TypeScript is not committed; the frontend
scripts that need it generate it first.
[Web application](docs/product/web-application.md#development-and-checks)
describes running the product locally, and
[Builds and releases](docs/delivery/builds-and-releases.md) covers cross builds
and release packages. [CONTRIBUTING.md](CONTRIBUTING.md) lists the rules a
change must follow.

## Extending

- **A new provider.** Add a provider crate that implements the provider
  contract for one vendor family; see
  [Add a provider](docs/guides/add-a-provider.md).

## License

[Apache-2.0](LICENSE).
