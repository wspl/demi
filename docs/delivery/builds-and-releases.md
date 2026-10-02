# Builds and releases

Trying a runner change on an x86_64 Linux machine that is also its Cloud's
execution host needs one Go target, `linux/amd64`. Build and package that
target, refresh the local Cloud image, and accept the change on both Hosts.
A published release carries every target of its executable.

The backend, machine manager, runner, and command programs are Go executables
in one module. A developer's Mac builds all six targets with Go itself.
`go run ./tools/release` builds and packages releases, pins Chrome for
Testing, and assembles the Cloud image.
[Package responsibilities](../architecture/crates-and-packages.md) defines
who owns each program; this document defines how it is delivered.

```text
go run ./tools/release native build     executable artifacts per target
                      |
                      v
go run ./tools/release native package   one release directory per executable
                      |
                      +-- runner release -------> backend: installers, downloads
                      +-- command packages -----> backend: object storage or development store
                      +-- Linux runner/packages -> Cloud image packaging on Linux
                      +-- backend --------------> Linux server
                      +-- machine manager ------> Cloud host service
```

## Executables and targets

The release target identifiers remain stable. The release tool maps them to
Go's `GOOS` and `GOARCH`; no cross compiler or platform SDK is needed.

| Platform | Release target | Go target (`GOOS/GOARCH`) |
| --- | --- | --- |
| macOS arm64 | `aarch64-apple-darwin` | `darwin/arm64` |
| macOS x86_64 | `x86_64-apple-darwin` | `darwin/amd64` |
| Linux arm64 | `aarch64-unknown-linux-musl` | `linux/arm64` |
| Linux x86_64 | `x86_64-unknown-linux-musl` | `linux/amd64` |
| Windows arm64 | `aarch64-pc-windows-msvc` | `windows/arm64` |
| Windows x86_64 | `x86_64-pc-windows-msvc` | `windows/amd64` |

Each executable is built for the targets where it runs:

| Executable | Targets | Reason |
| --- | --- | --- |
| `demi-runner` | All six | Paired devices run macOS, Linux, or Windows on arm64 or x86_64, and the Cloud guest runs Linux |
| `demi-file`, `demi-browser`, `demi-claude-code` | All six | A published command package supplies its operations on every target ([Publish a complete release](../execution/native-runtime.md#publish-a-complete-release)) |
| `demi-backend` | `linux/arm64`, `linux/amd64`, `darwin/arm64`, `darwin/amd64` | Servers run Linux; developers also run the backend on a Mac, with the Cloud in a Lima VM ([Develop on a Mac with Lima](../guides/mac-development.md)) |
| `demi-machine-manager` | `linux/arm64`, `linux/amd64` | The manager drives gVisor, Linux namespaces, cgroups, loop devices, and nftables |

Linux executables are statically linked and need no host C library, including
inside the Cloud guest. Windows executables need no separately installed C
runtime. The target identifiers describe the release protocol, not a C
linking requirement. Cross compilation does not accept kernel behavior or
packaged resources: release acceptance executes them on their platforms.

The release tool is not a product release. It runs on the developer's machine;
for image assembly, cross-build it for the Linux builder's architecture and
run it there ([guest image build](../../cloud-guest-image/README.md)):

```sh
CGO_ENABLED=0 GOFLAGS=-mod=readonly GOOS=linux GOARCH=arm64 \
  go build -trimpath -ldflags='-s -w' \
  -o .cache/release-linux-arm64 ./tools/release
```

## Toolchain

`go.mod` pins Go 1.27.1 through `toolchain go1.27.1`, for development and
releases. Developers and CI use that version; move the pin in a change of its
own that passes [Validation](#validation). The module's `toolchain` directive
selects the default toolchain, so also check `go version` in the build
environment rather than silently using a newer installed version.

**Open item: minimum macOS version.** The installed Go 1.27.1 toolchain's
`doc/` files and `go doc runtime` do not establish its macOS support floor.
Confirm that floor before specifying the minimum supported macOS release;
a successful cross-build alone does not establish runtime support.

Every product build uses `CGO_ENABLED=0`, including the backend's
`modernc.org/sqlite` driver through `database/sql`. No C compiler is needed.
The tree watch's macOS FSEvents file uses purego; that narrow native API call
does not enable cgo. The Linux race-test exception is described in
[Testing](testing.md#race-detection).

During the migration, set these for every command below, including `go run`,
`go generate`, and the Go commands launched by scripts:

```sh
export CGO_ENABLED=0
export GOFLAGS=-mod=readonly
```

`GOFLAGS=-mod=readonly` prevents the reference code's root `vendor/` directory
from selecting Go vendor mode. Remove that temporary requirement when the
migration removes the directory. Module and checksum changes are reviewed,
not silently made by a build.

Generated Go contracts are committed. `go generate` invokes
`tools/contractgen` to regenerate decoders, encoders, validation, and Zod from
marked Go types; frontend scripts invoke generation through `bun run contracts`
([Generated TypeScript](../architecture/contracts.md#generated-typescript)).
Building Go programs needs no frontend tools and does not regenerate contracts.

## Build profiles

The release tool owns release flags: `-trimpath -ldflags='-s -w'`. They remove
local source paths and strip symbol and debug tables, reducing what every
paired device downloads and every Cloud image carries. Development builds
keep Go's normal debug information. Do not add per-machine compiler flags;
adopt an optimization only with a measurement recorded beside its setting.
Go's build cache handles incremental compilation; keep it between builds.

A panic in a shell utility must stay within its job
([Shell jobs](../execution/runner.md#shell-jobs)); a resident command service
fails one invocation, not every conversation it holds. Recovery belongs at
the owning goroutine's boundary; no panic crosses a package boundary.

The sp11 matrix measured these stripped **skeleton sizes as a lower bound**
on functionality, not a production size promise (MiB = 1,048,576 bytes):

| Skeleton | darwin/amd64 | darwin/arm64 | linux/amd64 | linux/arm64 | windows/amd64 | windows/arm64 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Runner | 11.35 | 10.55 | 11.09 | 10.31 | 11.54 | 10.49 |
| Backend | 19.96 | 19.01 | 19.82 | 18.81 | — | — |
| Browser | 6.98 | 6.59 | 6.79 | 6.44 | 7.01 | 6.48 |
| Machine manager | — | — | 4.18 | 4.00 | — | — |

These retain representative library operations, not the full applications.
The backend skeleton links two SQLite engines; the product chooses modernc.
Browser sizes exclude Chrome and its resources. No file or Claude Code
program size was measured. The same 18 builds took 64.224 seconds with an
initially empty build cache and 5.499 seconds unchanged, with dependencies
already downloaded. These are single observations on the development Mac,
not build budgets or evidence of production size or performance gains.
[Migration spike results](go-migration.md#spike-results) indexes the evidence.

## Cross builds

Build on the developer's machine with the pinned Go toolchain:

```sh
go run ./tools/release native build \
  --target aarch64-apple-darwin --target aarch64-unknown-linux-musl
```

Without `--package` or `--target`, the release tool builds the runner and
command programs for all six targets. Repeated `--package <executable>`
options select programs, including the backend and machine manager. Repeated
`--target <release-target>` options select targets from the mapping above;
without them each selected executable gets every target it supports. An
unsupported program/target pair is refused before any build starts.
`--artifacts` selects the output directory, defaulting to `.cache/native-target`.

Development builds and packages only the targets of the Hosts in use. A
published release builds every supported target. There is no separate build
container or cross-tool installation step; the Linux image assembler still
needs its Linux host tools as specified by the image contract.

## Packaging

`go run ./tools/release native package` turns built executables into a release
directory.
It takes the same repeated `--target` options as the build and packages exactly
those targets; without them it requires every target of the executable.
`--artifacts` names the artifact directory the build wrote, with the
build's default.

```sh
go run ./tools/release native package --package demi-file \
  --artifacts .cache/native-target --output .cache/releases/demi-file-<version>
go run ./tools/release native package --package demi-browser \
  --artifacts .cache/native-target --output .cache/releases/demi-browser-<version>
go run ./tools/release native package --package demi-claude-code \
  --artifacts .cache/native-target --output .cache/releases/demi-claude-<version>
go run ./tools/release native package --package demi-runner \
  --artifacts .cache/native-target --output .cache/releases/runners
go run ./tools/release native package --package demi-backend \
  --artifacts .cache/native-target --output .cache/releases/demi-backend-<version>
```

Each executable has its own kind of release:

- **Command packages.** Each command program is released on its own. Its
  release directory holds `descriptor.json` and one subdirectory per target
  with the executable. The descriptor's id and operations are the ones the
  package's contract declares (`internal/cmdpkg/file/fileop` for `demi-file`,
  `internal/cmdpkg/browser/browserop` for `demi-browser`, and
  `internal/cmdpkg/claudecode/claudecodeop` for `demi-claude-code`), the operation
  list the program routes by, so a release
  cannot advertise an operation the program does not serve; its version is
  the workspace version. A package that needs resources gets them from the
  record its contract package keeps: `demi-browser`'s release carries the
  pinned Chrome for Testing archive of each packaged target that has one,
  as `resources/<sha256>`, and its descriptor names it as the resource
  `chrome`. Packaging downloads each archive it lacks into
  `.cache/resources/<sha256>`, checks it against the record, and copies it
  from there, so packaging again downloads nothing.
  [Bind an exact package](../execution/native-runtime.md#bind-an-exact-package)
  defines the descriptor.
- **Runner.** A runner release is a directory named by the hash of its
  contents, holding `manifest.json` and one executable per target. Once that
  directory is in place, packaging replaces the top-level `manifest.json`
  atomically, so it names the release packaged last; earlier releases stay for
  the runners installed from them.
- **Backend and machine manager.** Each is released as one executable per
  target that carries the workspace version
  ([Package versioning](package-versioning.md)). Its release
  directory holds `release.json` and one subdirectory per target with the
  executable. `release.json` names the executable, the version, and each
  target's executable SHA-256 and byte size, the same entry as a
  descriptor's `targets`:

  ```json
  {
    "executable": "demi-machine-manager",
    "version": "0.1.3",
    "targets": {
      "x86_64-unknown-linux-musl": { "sha256": "<SHA-256 in hex>", "size": 8523528 }
    }
  }
  ```

  The byte size above illustrates the record; it is not a measured Go size.
  No Demi program reads the record; it tells whoever copies the executable
  to a server what to check the copy against. Since a packaged version is
  immutable (below), a development build of an unchanged version goes to a
  directory of its own.

Every release is published the same way, through the one verified publication
of the artifact library
([Package responsibilities](../architecture/crates-and-packages.md)): stage
every artifact, verify each copy's size and SHA-256, publish once, and refuse
conflicting metadata or corrupt bytes already in place. A release already in
place with the same record and bytes is the one being published, so packaging
the same build again succeeds. A failed publication removes its temporary
files and leaves the top-level pointer as it was.

A release of fewer than all targets is a development release, for a backend
on the developer's own machine; publication to object storage refuses it, and
the backend's development store loads it
([Publish a complete release](../execution/native-runtime.md#publish-a-complete-release)).
A published version is immutable: publishing different artifacts needs a new
workspace version, or, for a development release, removing its directory
before packaging it again.

The backend loads the deployed command package releases and supplies the
selected descriptors and artifact locations to runners; publishing writes no
release catalog into application source. A deployment names the command
releases and their object store in `DEMI_NATIVE_CONFIG` and the runner release
directory in `DEMI_RUNNER_RELEASE_DIR`
([Backend deployment configuration](../execution/native-runtime.md#backend-deployment-configuration)).
The backend publishes command artifacts to object storage before it accepts
requests, and runners download them from storage through signed HTTPS URLs. A
backend on the developer's own machine loads development releases into its
development store instead and serves their executables itself
([Development backend](../backend/backend.md#development-backend)).

The Cloud image embeds a Linux runner release and the command package releases.
`go run ./tools/release cloud-image package` assembles the image on a Linux
builder of the image's architecture: [Cloud images](../cloud/images.md) defines the image, and
the [guest image build](../../cloud-guest-image/README.md) gives the steps.

## Chrome for Testing

Each Demi release pins one Chrome for Testing version
([Browser distribution](../browser/browser.md#browser-distribution)).
`go run ./tools/release browser-release` pins the version it is given:

```sh
go run ./tools/release browser-release 153.0.8010.36
```

It reads that version's official download metadata and, for each platform
Demi supports, downloads the `chrome` archive from Chrome for Testing's
download host through the artifact library, measures its size and SHA-256, and
checks that it holds the executable the record names. It then writes the
release record, `internal/cmdpkg/browser/browserop/release/chrome.json`, from which
packaging writes the `chrome` resource of `demi-browser`'s releases; commit it
with the change that adopts the version. Chrome for Testing publishes no
Windows arm64 build, so the record carries five of the six targets. The
downloads are not kept: packaging downloads each archive again and checks it
against the record.

## Validation

[Testing](testing.md) defines what a test protects. Developers and CI run the
same commands below with the environment in [Toolchain](#toolchain). There is
currently no hosted CI; that does not change the required checks. Release
acceptance runs the shared Go suites on each platform that ships a feature.

| Command | What it checks |
| --- | --- |
| `go vet ./...` | Static checks of host-buildable packages and tests |
| `gofmt -l cmd internal tools` | Formatting; output must be empty (`gofmt -w` applies it) |
| `go test ./...` | Package tests, including repository-program scenarios; `go test ./internal/gates -run <pattern>` selects one behavior while editing |
| `golangci-lint run ./...` | Pinned v2.14.0, with `staticcheck`, `errcheck`, `govet`, and `revive`; no warnings; a local suppression names its reason |
| `go run ./tools/archcheck` | Declared package dependency direction, including test imports and otherwise unused packages |
| `go run ./tools/cgocheck` | No selected cgo dependencies and no failed package loads on every shipping target, always with `CGO_ENABLED=0` |
| `go-check-sumtype -default-signifies-exhaustive=false ./...` | Exhaustive switches over annotated sealed interfaces; a default does not excuse a missing variant |
| `bash scripts/gomig/check.sh <package>...` | During migration: builds on shipping targets, vet, lint, cgo, imports, exhaustiveness, and race tests of the packages and their importers |
| `bun run test` | TypeScript tests, package boundaries, web app contract scenarios, and capture-extension JavaScript tests |

The release target table drives builds and checks. Run lint, import and
exhaustiveness checks under each applicable `GOOS`/`GOARCH`, as well as host
checks; a Mac run alone cannot inspect Linux-only manager code. Pin the
standalone sumtype tool to v0.5.0. Check acceptance-tagged packages with vet,
lint, imports, and exhaustiveness too; leaving those files out of the ordinary
build must not exempt them. Run the race suite on its execution host:

```sh
# macOS
CGO_ENABLED=0 go test -race ./...

# Linux; the test binary alone uses the C race runtime.
CGO_ENABLED=1 go test -race -tags netgo,osusergo ./...
```

### Programs used by tests

Backend scenarios with real runners, runner suites, and command-program suites
run in the default `go test ./...`; they need no manual prebuild. One shared
test-support package, `internal/programtest`, supplies the repository's own
programs. When `DEMI_TEST_PROGRAMS` is unset, it builds each requested program
once per test binary from the module with `go build -o <temporary dir>`.
Go's build cache makes repeat builds take seconds. The package owns the
shared temporary directory until the test binary's users have finished, then
removes it; individual tests still stop and wait for the processes they start.
Builds use `CGO_ENABLED=0` even when the calling test uses Linux's race runtime,
and `GOFLAGS=-mod=readonly` during the migration.

When `DEMI_TEST_PROGRAMS` is set, `internal/programtest` uses that directory
instead of building. It resolves program names with `.exe` on Windows and
fails clearly if a requested executable is absent; it never silently rebuilds
an explicitly supplied program. TypeScript suites and release acceptance set
this variable to test the intended artifacts. For example, on a Mac:

```sh
mkdir -p .cache/test-programs
go build -o .cache/test-programs/ ./cmd/...
export DEMI_TEST_PROGRAMS="$PWD/.cache/test-programs"
go test -tags acceptance -count=1 ./...
```

The command builds the programs available on the current platform, including
fixture programs; Linux also builds the machine manager. `bun run test` uses
an explicitly supplied `DEMI_TEST_PROGRAMS` directory and otherwise prepares
those programs once; to run selected TypeScript tests, build first and set
the same variable. Test fixture and scripted-machine support stays out of
product binaries.

Only suites that need resources outside the repository use `acceptance`:
real Chrome, the Claude Code CLI, and a real Cloud. They also skip unless
their required environment is supplied. The commands below use the prebuilt
directory above; their test names select each resource suite:

```sh
DEMI_TEST_CHROME=<chrome> \
  go test -tags acceptance -count=1 -p=1 -parallel=1 \
  ./internal/cmdpkg/browser/... -run Chrome
DEMI_TEST_CHROME=<chrome> \
  go test -tags acceptance -count=1 ./internal/backend/... -run RealBrowser
DEMI_TEST_CLAUDE_CODE=<claude> SSL_CERT_FILE=<suite-distribution-ca.pem> \
  go test -tags acceptance -count=1 ./internal/backend/... -run ClaudeCode
```

[Scenarios](scenarios.md) owns the suite behavior and resource configuration.
The Chrome tests run outside an execution sandbox, on macOS and in Linux,
as an ordinary user; Chrome's own sandbox stays enabled. Linux Chrome refuses
root. `DEMI_TEST_CHROME` names the executable of the pinned unpacked Chrome
for Testing release, with its full installation readable by that user. The
browser program receives that path as it would from a runner; backend
scenarios install it in the runner's artifact cache, with hard links where
available ([Install artifacts](../execution/native-runtime.md#install-artifacts)).
No test downloads Chrome or uses the user's home. Live-view tests decode H.264
with WebCodecs in that Chrome, as the page does. Cross-build success does not
replace these checks or acceptance of the extension and browser cleanup.

The Cloud suite runs on Linux as root, with a real manager and image, and
runs serially because backend reconciliation stops the manager's Clouds.
With the four resource variables in [Cloud suite](scenarios.md#cloud-suite)
set in the root test environment, run:

```sh
go test -tags acceptance -count=1 -p=1 -parallel=1 -v \
  ./internal/backend/... -run RealCloud
```

The stand-in execution-host harness must also support a machine without an
installed manager, disable manager resource limits for the suite, and clean
up and audit all processes, mounts, loop devices, network state, and listeners
on success, failure, and interruption, as specified in that scenario contract.
A Mac runs Linux manager tests in Lima
([Machine manager tests](../guides/mac-development.md#machine-manager-tests)).
Privileged manager tests carry `acceptance`; compile their test binaries as
an ordinary user, then run those binaries as root with the needed environment.
Each test owns its mount and network namespaces. Ordinary package tests
require neither root nor a real manager. Release acceptance separately checks
Linux amd64 and arm64 and the resource limits the stand-in leaves out
([Verification](../cloud/managed-hosts.md#verification)).

The Claude Code suite runs as an ordinary user, as the Cloud's runner does;
the CLI refuses the provider's permission mode as root. The local distribution
uses HTTPS. `SSL_CERT_FILE` names its fixture CA only for the test and its
children; the backend downloader and Cloud command program must both trust it
through Go's certificate-root loading. Keep the fixture certificate and
server certificate, whose keys are not regenerated during a run and whose
expiry is 2126-09-03. The CA private key was discarded. The real CLI calls a
local scripted endpoint with a made-up token and telemetry disabled, never a
real model ([Claude Code suite](scenarios.md#claude-code-suite)).

Release acceptance also checks the artifacts and installers on each platform:

- A Linux executable with an ELF interpreter or shared-library dependency
  fails acceptance.
- Windows exercises the PowerShell installer; Linux and macOS exercise the
  shell installer, including registration separation, release reuse, and
  draining upgrades.
- Each installer and publication fixture runs three times per platform to
  exercise repeated startup and teardown; disable test-result caching for
  these repeats.
- The manual real-machine checks in
  [Scenarios](scenarios.md#real-machine-acceptance) cover what automation leaves
  out, including the Cloud conversation browser's live view.

Synthetic service/client benchmarks belong to the command SDK's Go benchmarks.
Run them explicitly with `go test -run '^$' -bench . ./internal/cmdsdk`; they
measure the current machine and build and never call a model. They are outside
the regular suite.

## Cloud image refresh

The Cloud runs the Linux target that matches its execution host. Build and
package that target together with the paired-device target used for acceptance.
`go run ./tools/release cloud-image package` assembles the image on its Linux
builder. The [Cloud image contract](../cloud/images.md#acceptance-and-local-refresh)
owns embedding, manager restart, local reset, and checking running artifact
identities; rebuilding a native release alone does not refresh a pinned Cloud.
A manager change is built for its host's Linux target and installed with the
service ([Cloud setup](../cloud/setup.md)).

The shell now uses the Host's system utilities. The following user-facing
consequences remain open work after the migration, per
[owner decision 4](go-migration.md#owner-decisions): tracking writes by
`sed -i`, `tee`, and `sort -o`; the BSD utilities of a paired Mac; Windows
devices without Unix utilities; including GNU coreutils, findutils, diffutils,
sed, grep, ripgrep, and jq in the Cloud image; and telling the model which
shell, platform, and utilities a job has. Cross-build or image acceptance must
not be reported as resolving those items.
