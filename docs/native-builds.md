# Native builds and releases

`scripts/native/build.ts` builds `demi-runner` and `demi-commands` for the six
triples in `command-protocol`. The release environment is Linux, including Apple
and Windows cross-compilation. Host platform execution is a separate CI gate.

The first-party Cargo workspace contains `crates/runner`,
`crates/command-service` and `crates/demi-commands`. Runner and Demi commands
produce executables; command-service is their shared library. TypeScript protocol
packages are build inputs, not additional Cargo workspace members.

## Toolchain

`rust-toolchain.toml` and `scripts/native/Dockerfile` pin the build tools.

Cargo runs contract generation through each consuming crate's `build.rs`.
Generated Rust types and validation code go into `OUT_DIR` and are included by
the crate; there is no manual source-generation prerequisite or committed
generated contract. Zod schemas in the owning TypeScript packages are the sole
authority. Runner generates message and manifest bindings; command-service
generates command wire and package bindings. A build script may invoke the shared
JS/TS generation tooling in `scripts/`. See
[the runtime design](demi-next/native-runtime.md#contract-generation-and-validation)
for direct Rust validation requirements.

The Dockerfile installs the Linux build tools. Supply an Apple SDK directory;
the script validates its SDK metadata before building. `ring` chooses `clang`
on Windows arm64, so the build explicitly selects the MSVC driver dialect and
release optimization to match cargo-xwin's SDK flags. The container target path
is `/build`, avoiding clang-cl's interpretation of `/output` as an output flag.

```sh
docker build -t demi-native-tools -f scripts/native/Dockerfile .
bun --conditions development scripts/native/build.ts \
  --container demi-native-tools \
  --sdk /path/to/MacOSX15.4.sdk
```

Use repeated `--target <triple>` options for a subset. `--artifacts` selects the
Cargo target directory; its default is `.cache/native-target`.

## Packaging

```sh
bun --conditions development scripts/native/release-package.ts \
  --artifacts .cache/native-target --output .cache/releases/demi-builtin-0.1.0
bun --conditions development scripts/native/release-runner.ts \
  --artifacts .cache/native-target --output .cache/releases/runners
```

The command package directory contains its runtime release descriptor and six
target subdirectories. The backend loads deployed releases and supplies the
selected descriptors and artifact locations to runners. Publishing does not
write a release catalog into application source. A version is immutable: choose
a new package version when publishing different artifacts.

Runner packaging creates a hash-named directory containing `manifest.json` and
six executables, then atomically advances the top-level manifest. Both packagers
use `scripts/native/release-files.ts`: stage every artifact, verify copied size
and SHA-256, publish once, and refuse conflicting metadata or corrupted existing
bytes. Failed publication removes its temporary files and leaves the pointer.

Backend `DEMI_NATIVE_CONFIG` selects complete package releases and an S3/OSS
store. `DEMI_RUNNER_RELEASE_DIR` selects the runner download directory. See the
[deployment configuration](demi-next/native-runtime.md#backend-deployment-configuration).
The backend publishes command artifacts before accepting requests, while runners
download signed HTTPS artifact URLs directly from storage.

## Validation

`.github/workflows/native.yml` executes the shared Rust suite on all six native
OS/architecture combinations. Linux jobs use musl and reject an ELF interpreter
or shared-library dependency. Windows uses static CRT and exercises the
PowerShell installer; Unix exercises the shell installer. These checks include
registration separation, release reuse and draining upgrades.
Each installer and publication fixture runs three times per platform job to
exercise repeated process startup and teardown.

`crates/command-service/examples/benchmark.rs` is a standalone synthetic service and
client. Build it with `cargo build --release -p demi-command-service --example
benchmark`, then execute `target/release/examples/benchmark`. It never calls a
model. Run it directly to obtain measurements for the current machine and build.
