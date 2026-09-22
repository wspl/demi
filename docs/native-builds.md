# Native builds and releases

`scripts/native/build.ts` builds `demi-runner` and the command packages
(`demi-commands`, `demi-claude`) for the six triples in `command-protocol`, or
for the ones named with `--target`; repeated `--package <crate>` options build
only those crates. The same
cross tools run on Linux and macOS. Host platform execution is a separate gate.

The first-party Cargo workspace contains `crates/runner`,
`crates/command-service`, `crates/demi-commands` and `crates/demi-claude`. The
runner and the command packages produce executables; command-service is their
shared library. TypeScript protocol
packages are build inputs, not additional Cargo workspace members.

## Toolchain

`rust-toolchain.toml` pins the Rust toolchain and lists the six targets, which
rustup installs with it. `scripts/native/Dockerfile` pins the cross tools.

Cargo runs contract generation through each consuming crate's `build.rs`.
Generated Rust types and validation code go into `OUT_DIR` and are included by
the crate; there is no manual source-generation prerequisite or committed
generated contract. Zod schemas in the owning TypeScript packages are the sole
authority. Runner generates message and manifest bindings; command-service
generates command wire and package bindings. A build script may invoke the shared
JS/TS generation tooling in `scripts/`. See
[the runtime design](demi-next/native-runtime.md#contract-generation-and-validation)
for direct Rust validation requirements.

The tools are cargo-zigbuild with Zig for the Apple and Linux targets, and
cargo-xwin with LLVM for the Windows targets. Install them on the build machine
at the versions the Dockerfile pins; on macOS:

```sh
brew install zig@0.15 llvm lld
cargo install --locked cargo-zigbuild --version 0.23.4
cargo install --locked cargo-xwin --version 0.23.1
```

Homebrew does not link these formulae. `build.ts` takes Zig from
`CARGO_ZIGBUILD_ZIG_PATH` and the LLVM tools from `PATH`:

```sh
export CARGO_ZIGBUILD_ZIG_PATH="$(brew --prefix zig@0.15)/bin/zig"
export PATH="$(brew --prefix llvm)/bin:$(brew --prefix lld)/bin:$PATH"
```

Build on the machine itself. Supply an Apple SDK directory for the Apple
targets; the script validates its SDK metadata before building. `ring` chooses
`clang` on Windows arm64, so the build explicitly selects the MSVC driver
dialect and release optimization to match cargo-xwin's SDK flags.

```sh
bun --conditions development scripts/native/build.ts \
  --sdk /Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk
```

Use repeated `--target <triple>` options for a subset. `--artifacts` selects the
Cargo target directory; its default is `.cache/native-target`.

Development builds only the targets of the Hosts in use, and packages a release
of the same targets (see [Packaging](#packaging)). Building six targets to try
a change on one machine is wasted time:

```sh
bun --conditions development scripts/native/build.ts \
  --sdk /Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk \
  --target aarch64-apple-darwin --target aarch64-unknown-linux-musl
```

`--container <image>` runs the same build inside the image the Dockerfile
describes, for a machine without the tools. A bind-mounted checkout is slow
there, and the container and the machine do not share a Cargo target directory,
so prefer the machine's own tools. The container target path is `/build`,
avoiding clang-cl's interpretation of `/output` as an output flag.

```sh
docker build -t demi-native-tools -f scripts/native/Dockerfile .
bun --conditions development scripts/native/build.ts \
  --container demi-native-tools --sdk /path/to/MacOSX15.4.sdk
```

## Packaging

```sh
bun --conditions development scripts/native/release-package.ts --package demi-commands \
  --artifacts .cache/native-target --output .cache/releases/demi-builtin-0.1.0
bun --conditions development scripts/native/release-package.ts --package demi-claude \
  --artifacts .cache/native-target --output .cache/releases/demi-claude-0.1.0
bun --conditions development scripts/native/release-runner.ts \
  --artifacts .cache/native-target --output .cache/releases/runners
```

Each command package is released on its own: `--package` names its crate, and
the release carries that crate's package id and operations. Both packagers take
the same repeated `--target <triple>` options as the build
and package exactly those targets; without them they require all six. A release
of fewer targets is a development release: the backend artifact module refuses
to publish it
([Publish a complete release](demi-next/native-runtime.md#publish-a-complete-release)).

The command package directory contains its runtime release descriptor and one
subdirectory per target. The backend loads deployed releases and supplies the
selected descriptors and artifact locations to runners. Publishing does not
write a release catalog into application source. A version is immutable: choose
a new package version when publishing different artifacts.

Runner packaging creates a hash-named directory containing `manifest.json` and
one executable per target, then atomically advances the top-level manifest. Both packagers
use `scripts/native/release-files.ts`: stage every artifact, verify copied size
and SHA-256, publish once, and refuse conflicting metadata or corrupted existing
bytes. Failed publication removes its temporary files and leaves the pointer.

Backend `DEMI_NATIVE_CONFIG` selects complete package releases and an S3
store. `DEMI_RUNNER_RELEASE_DIR` selects the runner download directory. See the
[deployment configuration](demi-next/native-runtime.md#backend-deployment-configuration).
The backend publishes command artifacts before accepting requests, while runners
download signed HTTPS artifact URLs directly from storage.

## Validation

There is no hosted CI. The shared Rust suite is run on each platform that
ships a feature as part of release acceptance. Linux builds use musl and reject
an ELF interpreter or shared-library dependency. Windows uses static CRT and
exercises the PowerShell installer; Unix exercises the shell installer. These
checks include registration separation, release reuse and draining upgrades.
Each installer and publication fixture is run three times per platform to
exercise repeated process startup and teardown.

`crates/command-service/examples/benchmark.rs` is a standalone synthetic service and
client. Build it with `cargo build --release -p demi-command-service --example
benchmark`, then execute `target/release/examples/benchmark`. It never calls a
model. Run it directly to obtain measurements for the current machine and build.

## Cloud image refresh

Cloud runs the Linux target matching its execution host, including arm64 inside
Lima on Apple silicon. Build and package that target with the paired-device target
used for acceptance. The [Cloud image contract](cloud-images.md#acceptance-and-local-refresh)
owns embedding, manager restart, local reset, and checking the executing artifact
identities; rebuilding a native release alone does not refresh a pinned Cloud.
