# Native builds and releases

`scripts/native/build.ts` builds `demi-runner` and `demi-commands` for the six
triples in `command-protocol`. The release environment is Linux, including Apple
and Windows cross-compilation. Host platform execution is a separate CI gate.

## Toolchain

- Rust 1.98.1, as pinned in `rust-toolchain.toml`.
- Zig 0.15.2 and cargo-zigbuild 0.23.4 for Linux musl and macOS.
- cargo-xwin 0.23.1 with LLVM for Windows MSVC, static CRT linkage.
- Apple SDK 15.4, with macOS deployment minimum 13.0.
- Windows SDK 10.0.26100 and CRT package 14.44.17.14.

The Dockerfile installs the Linux build tools. Supply an Apple SDK directory;
the script validates its SDK metadata before building. `ring` chooses `clang`
on Windows arm64, so the build explicitly selects the MSVC driver dialect and
release optimization to match cargo-xwin's SDK flags. The container target path
is `/build`, avoiding clang-cl's interpretation of `/output` as an output flag.

```sh
docker build -t demi-native-tools:rust-1.98.1 -f scripts/native/Dockerfile .
bun --conditions development scripts/native/build.ts \
  --container demi-native-tools:rust-1.98.1 \
  --sdk /path/to/MacOSX15.4.sdk
```

Use repeated `--target <triple>` options for a subset. `--artifacts` selects the
Cargo target directory; its default is `.cache/native-target`.

## Packaging

```sh
bun --conditions development packages/demi-package/scripts/release.ts \
  --artifacts .cache/native-target --output .cache/releases/demi-builtin-0.1.0
bun --conditions development packages/runner/runtime/release.ts \
  --artifacts .cache/native-target --output .cache/releases/runners
```

The command package directory contains `descriptor.json` and six target
subdirectories. The script also updates the public `demiPackage` descriptor in
`packages/demi-package/src/release.json` to identify those exact bytes. A version
is immutable: choose a new package version when publishing different artifacts.

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

`command-service/examples/benchmark.rs` is a standalone synthetic service and
client. Build it with `cargo build --release -p demi-command-service --example
benchmark`, then execute `target/release/examples/benchmark`. It never calls a
model. Execution evidence belongs in `docs/demi-next/progress.md`; a compiled
binary or a workflow definition alone does not establish target runtime acceptance.
