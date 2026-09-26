# Native builds and releases

`scripts/native/build.ts` builds the Go programs that run on Hosts:
`demi-runner` and the command packages (`demi-commands`, `demi-claude`), for
the six targets in `command-protocol`, or for the ones named with `--target`;
repeated `--package <program>` options build only those programs. Host platform
execution is a separate gate.

The programs are `cmd/demi-runner`, `cmd/demi-commands` and `cmd/demi-claude`
in the repository's Go module; `internal/commandservice` is their shared
communication SDK. TypeScript protocol packages are build inputs through
generated contracts, not Go packages.

## Toolchain

`go.mod` pins the Go version. Every program is pure Go and builds with
`CGO_ENABLED=0`, so one machine cross-compiles every target with the Go
toolchain alone; no C toolchain, SDK or container is involved. The target
identifiers stay the ones `command-protocol` defines, since manifests and
release descriptors carry them; the build maps each to a Go platform:

| Target | `GOOS/GOARCH` |
| --- | --- |
| `aarch64-apple-darwin` | `darwin/arm64` |
| `x86_64-apple-darwin` | `darwin/amd64` |
| `aarch64-unknown-linux-musl` | `linux/arm64` |
| `x86_64-unknown-linux-musl` | `linux/amd64` |
| `aarch64-pc-windows-msvc` | `windows/arm64` |
| `x86_64-pc-windows-msvc` | `windows/amd64` |

The build runs contract generation (`bun run go:contracts`) first. See
[native-runtime.md](demi-next/native-runtime.md#contract-generation-and-validation)
for validation requirements.

```sh
bun --conditions development scripts/native/build.ts
```

`--artifacts` selects the output directory; its default is
`.cache/native-target`. Development builds only the targets of the Hosts in
use, and packages a release of the same targets (see [Packaging](#packaging)):

```sh
bun --conditions development scripts/native/build.ts --target x86_64-unknown-linux-musl
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

Each command package is released on its own: `--package` names its program, and
the release carries that program's package id and operations. Both packagers take
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

There is no hosted CI. The Go suite is run on each platform that ships a
feature as part of release acceptance. Linux artifacts are rejected if they
have an ELF interpreter or a shared-library dependency. Windows uses static CRT and
exercises the PowerShell installer; Unix exercises the shell installer. These
checks include registration separation, release reuse and draining upgrades.
Each installer and publication fixture is run three times per platform to
exercise repeated process startup and teardown.

## Cloud image refresh

Cloud runs the Linux target matching its execution host, including arm64 inside
Lima on Apple silicon. Build and package that target with the paired-device target
used for acceptance. The [Cloud image contract](cloud-images.md#acceptance-and-local-refresh)
owns embedding, manager restart, local reset, and checking the executing artifact
identities; rebuilding a native release alone does not refresh a pinned Cloud.
