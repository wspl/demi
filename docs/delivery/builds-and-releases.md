# Builds and releases

The backend, the machine manager, the runner, and the command programs are
Rust executables built from one Cargo workspace. A developer builds all of them
on their own machine: the cross tools installed there compile every target, and
`cargo xtask` runs the builds, packages the releases, pins the Chrome for
Testing release, and assembles the Cloud image.
[Crates and packages](../architecture/crates-and-packages.md#crates) lists the
crates; this document covers the executables, their targets, and their
releases.

For example, trying a runner change on an Apple silicon Mac whose Cloud runs in
Lima needs two targets: `aarch64-apple-darwin` for the Mac as a paired device,
and `aarch64-unknown-linux-musl` for the Cloud guest. The developer builds and
packages those two, refreshes the local Cloud image, and accepts the change on
both Hosts. A published release carries every target.

```text
cargo xtask native build     compiles each executable for its targets
        |                    into the Cargo target directory
        v
cargo xtask native package   one release directory per executable
        |
        +-- runner release ---------> backend: installers, runner downloads
        +-- command packages -------> backend: published to object storage
        +-- Linux runner, packages -> cargo xtask cloud-image package: Cloud image
        +-- backend ----------------> a Linux server, or a developer's Mac
        +-- machine manager --------> the Cloud host's service
```

## Executables and targets

The workspace builds for six targets, each with one cross tool:

| Platform | Target triples | Cross tool |
| --- | --- | --- |
| macOS | `aarch64-apple-darwin`, `x86_64-apple-darwin` | cargo-zigbuild with an Apple SDK |
| Linux | `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl` | cargo-zigbuild |
| Windows | `aarch64-pc-windows-msvc`, `x86_64-pc-windows-msvc` | cargo-xwin with LLVM and the Microsoft SDK |

Each executable is built for the targets where it runs:

| Executable | Targets | Reason |
| --- | --- | --- |
| `demi-runner` | All six | Paired devices run macOS, Linux, or Windows on arm64 or x86_64, and the Cloud guest runs Linux |
| `demi-commands`, `demi-claude` | All six | A published command package supplies its operations on every target ([Publish a complete release](../execution/native-runtime.md#publish-a-complete-release)) |
| `demi-backend` | `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl`, `aarch64-apple-darwin`, `x86_64-apple-darwin` | Servers run Linux; a developer also runs the backend on a Mac, with the Cloud in a Lima VM |
| `demi-machines` | `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl` | The machine manager drives gVisor, Linux namespaces, cgroups, loop devices, and nftables, which exist only on Linux |

Linux executables link musl statically, so one file runs on any distribution
and inside the Cloud guest, whatever C library the host has. Windows
executables link the C runtime statically, so they need no separately installed
runtime.

`xtask` itself is not released. It runs on the developer's machine. The Linux
builder of the Cloud image runs a Linux musl build of it, which the developer's
machine cross-compiles, for example with
`cargo zigbuild --release -p xtask --target aarch64-unknown-linux-musl` for an
arm64 builder ([guest image build](../../packages/guest-image/README.md)).

## Toolchain

`rust-toolchain.toml` pins the Rust toolchain and lists the six targets, which
rustup installs with it. The contract crates are ordinary Rust, and no build
script generates contract code or needs the frontend's tooling, so a Cargo build
needs only this toolchain and, for host builds, a C compiler for the
dependencies that compile C code. The browser's TypeScript contracts come from
a separate command, `xtask contracts`, which the frontend's scripts run through
`bun run contracts` before they need them
([Generated TypeScript](../architecture/contracts.md#generated-typescript)).

Cross builds use cargo-zigbuild with Zig for the Apple and Linux targets, and
cargo-xwin with LLVM for the Windows targets. With these tools one machine,
Linux or macOS, builds every target; no target needs a build machine of its own
platform. `scripts/native/Dockerfile` pins their versions. Install the same
versions on the build machine; on macOS:

```sh
brew install zig@<zig version> llvm lld
cargo install --locked cargo-zigbuild --version <cargo-zigbuild version>
cargo install --locked cargo-xwin --version <cargo-xwin version>
```

Homebrew does not link these formulae. `cargo xtask` takes Zig from
`CARGO_ZIGBUILD_ZIG_PATH` and the LLVM tools from `PATH`:

```sh
export CARGO_ZIGBUILD_ZIG_PATH="$(brew --prefix zig@<zig version>)/bin/zig"
export PATH="$(brew --prefix llvm)/bin:$(brew --prefix lld)/bin:$PATH"
```

`cargo xtask` is an alias, in the repository's `.cargo/config.toml`, for
`cargo run --package xtask --`. It selects one crate, so it builds `xtask` with
a copy of the dependencies of its own ([Validation](#validation)); after a
build of the whole workspace, `target/debug/xtask` runs the same commands
without that copy, as `bun run contracts` does.

`cargo xtask` pins the remaining inputs: the Apple SDK version, the Windows SDK
and C runtime versions that cargo-xwin downloads, and the minimum macOS
version. The Apple targets need an Apple SDK directory, passed with `--sdk` or
`SDKROOT`; `cargo xtask` checks its SDK metadata against the pin before
building. `ring` chooses `clang` on Windows arm64, so the build explicitly
selects the MSVC driver dialect and release optimization to match cargo-xwin's
SDK flags.

## Cross builds

Build on the machine itself, with its own cross tools; the build container
below is only for a machine that lacks them:

```sh
cargo xtask native build \
  --sdk /Library/Developer/CommandLineTools/SDKs/MacOSX<version>.sdk
```

Without `--package` or `--target` options, `cargo xtask native build` builds
the runner and the command programs for all six targets. Repeated
`--package <crate>` options name the executables to build, including
`demi-backend` and `demi-machines`. Repeated `--target <triple>` options name
the targets; without them each named executable is built for every target
[Executables and targets](#executables-and-targets) gives it. A named target
that a named executable does not run on, such as `demi-machines` for a macOS
target, is refused before anything builds. `--artifacts`
selects the Cargo target directory; its default is `.cache/native-target`.

Development builds only the targets of the Hosts in use, and packages a
release of the same targets (see [Packaging](#packaging)). Building six targets
to try a change on one machine is wasted time:

```sh
cargo xtask native build \
  --sdk /Library/Developer/CommandLineTools/SDKs/MacOSX<version>.sdk \
  --target aarch64-apple-darwin --target aarch64-unknown-linux-musl
```

`--container <image>` runs the same build inside the image the Dockerfile
describes, for a machine without the tools. A bind-mounted checkout is slow
there, and the container and the machine do not share a Cargo target
directory, so prefer the machine's own tools. The container's target path is
`/build`, because clang-cl reads `/output` as an output flag.

```sh
docker build -t demi-native-tools -f scripts/native/Dockerfile .
cargo xtask native build \
  --container demi-native-tools --sdk /path/to/MacOSX<version>.sdk
```

## Packaging

`cargo xtask native package` turns built executables into a release directory.
It takes the same repeated `--target` options as the build and packages exactly
those targets; without them it requires every target of the executable.
`--artifacts` names the Cargo target directory the build wrote, with the
build's default.

```sh
cargo xtask native package --package demi-commands \
  --artifacts .cache/native-target --output .cache/releases/demi-builtin-<version>
cargo xtask native package --package demi-claude \
  --artifacts .cache/native-target --output .cache/releases/demi-claude-<version>
cargo xtask native package --package demi-runner \
  --artifacts .cache/native-target --output .cache/releases/runners
cargo xtask native package --package demi-backend \
  --artifacts .cache/native-target --output .cache/releases/demi-backend-<version>
```

Each executable has its own kind of release:

- **Command packages.** Each command program is released on its own. Its
  release directory holds `descriptor.json` and one subdirectory per target
  with the executable. The descriptor's id and operations are the ones the
  package's contract crate declares (`builtin-protocol` for `demi-commands`,
  `claude-protocol` for `demi-claude`), the operation list the program routes
  by, so a release cannot advertise an operation the program does not serve;
  its version is the workspace version.
  [Bind an exact package](../execution/native-runtime.md#bind-an-exact-package)
  defines the descriptor.
- **Runner.** A runner release is a directory named by the hash of its
  contents, holding `manifest.json` and one executable per target. Once that
  directory is in place, packaging replaces the top-level `manifest.json`
  atomically, so it names the release packaged last; earlier releases stay for
  the runners installed from them.
- **Backend and machine manager.** Each is released as one executable per
  target that carries the workspace version
  ([Package versioning](package-versioning.md#rust-executables)). Its release
  directory holds `release.json` and one subdirectory per target with the
  executable. `release.json` names the executable, the version, and each
  target's executable SHA-256 and byte size, the same entry as a
  descriptor's `targets`:

  ```json
  {
    "executable": "demi-machines",
    "version": "0.1.3",
    "targets": {
      "x86_64-unknown-linux-musl": { "sha256": "<SHA-256 in hex>", "size": 8523528 }
    }
  }
  ```

  No Demi program reads the record; it tells whoever copies the executable
  to a server what to check the copy against. Since a packaged version is
  immutable (below), a development build of an unchanged version goes to a
  directory of its own.

Every release is published the same way, through the one verified publication
of the artifact library
([Crates and packages](../architecture/crates-and-packages.md#crates)): stage
every artifact, verify each copy's size and SHA-256, publish once, and refuse
conflicting metadata or corrupt bytes already in place. A release already in
place with the same record and bytes is the one being published, so packaging
the same build again succeeds. A failed publication removes its temporary
files and leaves the top-level pointer as it was.

A release of fewer than all targets is a development release, for a backend
on the developer's own machine; publication refuses it
([Publish a complete release](../execution/native-runtime.md#publish-a-complete-release)).
A published version is immutable: publishing different artifacts needs a new
workspace version.

The backend loads the deployed command package releases and supplies the
selected descriptors and artifact locations to runners; publishing writes no
release catalog into application source. A deployment names the command
releases and their object store in `DEMI_NATIVE_CONFIG` and the runner release
directory in `DEMI_RUNNER_RELEASE_DIR`
([Backend deployment configuration](../execution/native-runtime.md#backend-deployment-configuration)).
The backend publishes command artifacts to object storage before it accepts
requests, and runners download them from storage through signed HTTPS URLs.

The Cloud image embeds a Linux runner release and the command package releases.
`cargo xtask cloud-image package` assembles the image on a Linux builder of the
image's architecture: [Cloud images](../cloud/images.md) defines the image, and
the [guest image build](../../packages/guest-image/README.md) gives the steps.

## Chrome for Testing

Each Demi release pins one Chrome for Testing version
([Browser distribution](../browser/browser.md#browser-distribution)).
`cargo xtask browser-release` pins the version it is given:

```sh
cargo xtask browser-release 153.0.8010.36
```

It reads that version's official download metadata and, for each platform
Demi supports, downloads the `chrome` archive from Chrome for Testing's
download host through the artifact library, measures its size and SHA-256, and
checks that it holds the executable the record names. It then writes the
release record, `crates/builtin-protocol/src/release/chrome.json`, which
`demi-commands` compiles in and the Cloud image build installs from; commit it
with the change that adopts the version. Chrome for Testing publishes no
Windows arm64 build, so the record carries five of the six targets. The
downloads are not kept: every installer downloads its archive again and checks
it against the record.

## Validation

There is no hosted CI. Developers run the checks on their machines, and
release acceptance runs the shared Rust suite on each platform that ships a
feature.

Every Rust command but the Chrome suite's selects the same thing: the whole
workspace with the runner's `test-fixtures` feature, which builds the fixture
programs the tests start. Cargo unifies features over what one command
selects, so a command that selected one crate or other features would build
its own copy of every shared dependency.

| Command | What it runs |
| --- | --- |
| `cargo check --workspace --all-targets --features demi-runner/test-fixtures` | The type check of every crate, test and example |
| `cargo test --workspace --features demi-runner/test-fixtures` | The Rust tests, the crate boundary check among them ([Boundary checks](../architecture/crates-and-packages.md#boundary-checks)); `--test <name>` runs one test target |
| `DEMI_TEST_CHROME=<chrome> cargo test --workspace --features demi-runner/test-fixtures,demi-commands/testing --test browser -- --include-ignored --test-threads=1` | The tests that start Chrome, one at a time, with the executable of the pinned Chrome for Testing release; the selection adds the page-driving helpers of `demi-commands` |
| `bun run test` | The TypeScript tests, after it builds the programs they start with the same selection |

A test that starts another program, such as a runner or `demi-commands`,
starts the one Cargo built into the target directory the test runs from
(`command_service::testing::built_program`); it builds nothing itself.
`cargo test` of the whole selection builds every program first, while one
test target builds only what it links, so before running one alone that
starts another crate's program, build the selection with
`cargo build --workspace --all-targets --features demi-runner/test-fixtures`.
The TypeScript tests take the programs from `DEMI_TEST_PROGRAMS`, which
`bun run test` sets to that directory. An ordinary test run starts no Chrome;
release acceptance runs the Chrome tests. The machine manager builds only for Linux, so
on a Mac its tests are cross-built with cargo-zigbuild and run in the Lima VM
([Verification](../cloud/managed-hosts.md#verification)). The tests that need
root are ignored in an ordinary run; as root, `--include-ignored` runs them,
each in mount and network namespaces of its own:

```sh
cargo zigbuild --tests --target aarch64-unknown-linux-musl \
  -p demi-machines -p demi-machines-protocol --target-dir .cache/linux-target
limactl shell demi-machines -- <test executable>
limactl shell demi-machines -- sudo <test executable> --include-ignored
```
[Scenarios](scenarios.md) defines the suites that run the whole backend.

Release acceptance also checks the artifacts and installers on each platform:

- A Linux executable with an ELF interpreter or a shared-library dependency
  fails.
- Windows exercises the PowerShell installer; Linux and macOS exercise the
  shell installer. These checks include registration separation, release
  reuse, and draining upgrades.
- Each installer and publication fixture runs three times per platform to
  exercise repeated process startup and teardown.

`crates/command-service/examples/benchmark.rs` is a standalone synthetic
service and client. Build it with
`cargo build --release -p demi-command-service --example benchmark`, then run
`target/release/examples/benchmark` to measure the machine and build it runs on.
It never calls a model.

## Cloud image refresh

The Cloud runs the Linux target that matches its execution host, including
arm64 inside Lima on Apple silicon. Build and package that target together with
the paired-device target used for acceptance. The
[Cloud image contract](../cloud/images.md#acceptance-and-local-refresh) owns
embedding, manager restart, local reset, and checking the identities of the
running artifacts; rebuilding a native release alone does not refresh a pinned
Cloud. A change to the machine manager itself is built for its host's Linux
target and installed with the service ([Cloud setup](../cloud/setup.md)).
