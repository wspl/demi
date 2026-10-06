# Builds and releases

The backend, the machine manager, the runner, and the command programs are
Rust executables built from one Cargo workspace. `cargo xtask` runs the
builds, packages the releases, assembles a server release, pins the Chrome
for Testing release, and assembles the Cloud image. The same commands run in
two places:

- **On a developer's machine.** The cross tools installed there compile the
  targets of the Hosts in use, whatever the machine's own platform.
- **In the release workflow.** A published release is built on GitHub's
  hosted runners, each platform's targets on a runner of that platform
  ([Release workflow](#release-workflow)).

[Crates and packages](../architecture/crates-and-packages.md#crates) lists the
crates; this document covers the executables, their targets, and their
releases.

For example, trying a runner change on an x86_64 Linux machine that is also
its own Cloud's execution host needs one target, `x86_64-unknown-linux-musl`:
the machine runs it as a paired device, and so does the Cloud guest. The
developer builds and packages that target, refreshes the local Cloud image,
and accepts the change on both Hosts. A published release carries every
target.

```text
cargo xtask native build     compiles each executable for its targets
        |                    into the Cargo target directory
        v
cargo xtask native package   one release directory per executable
        |
        v
cargo xtask server-release   one server release for a Linux target: the
        |                    backend, the manager, the web app and the
        |                    packages' manifests, and the release's files,
        |                    each program for each target
        +-- the release and its files --> cloud-guest-image build: image/
        +-- the release ----------------> a Linux server, whose backend takes
                                          each program from the files when a
                                          Host first needs it
```

## Executables and targets

The workspace builds for six targets. A target of the building machine's own
platform builds with that platform's toolchain; a target of another platform
builds with a cross tool:

| Platform | Target triples | On its own platform | From another platform |
| --- | --- | --- | --- |
| macOS | `aarch64-apple-darwin`, `x86_64-apple-darwin` | Apple's compiler and linker with the pinned SDK, both architectures on one Mac | cargo-zigbuild with the pinned Apple SDK |
| Linux | `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl` | cargo-zigbuild | cargo-zigbuild |
| Windows | `aarch64-pc-windows-msvc`, `x86_64-pc-windows-msvc` | MSVC with the pinned toolset and SDK | cargo-xwin with LLVM and the Microsoft SDK |

Linux keeps cargo-zigbuild on Linux itself. Zig there is only the C compiler
and linker of the static musl build: the C library of rustls's provider,
`aws-lc-sys`, does not build reliably with the `musl-gcc` wrapper that Linux
distributions ship, and Zig carries its own musl. The release workflow builds
each Linux target on a runner of its own architecture, so nothing in a
release is built for an architecture other than the one that compiles it,
except `x86_64-apple-darwin`, which Apple's toolchain builds on an arm64 Mac
as it does for every Mac application.

Each executable is built for the targets where it runs:

| Executable | Targets | Reason |
| --- | --- | --- |
| `demi-runner` | All six | Paired devices run macOS, Linux, or Windows on arm64 or x86_64, and the Cloud guest runs Linux |
| `demi-file`, `demi-browser`, `demi-claude-code` | All six | A released command package supplies its operations on every target ([Publish a complete release](../execution/native-runtime.md#publish-a-complete-release)) |
| `demi-backend` | `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl`; `aarch64-apple-darwin`, `x86_64-apple-darwin` in development only | Servers run Linux, and a release carries the Linux targets; a developer may also run the backend on a Mac, with the Cloud in a Lima VM ([Develop on a Mac with Lima](../guides/mac-development.md)) |
| `demi-machine-manager` | `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl` | The machine manager drives gVisor, Linux namespaces, cgroups, loop devices, and nftables, which exist only on Linux |
| `demi-server` | `aarch64-unknown-linux-musl`, `x86_64-unknown-linux-musl` | It installs, upgrades and rolls back a server, which runs Linux ([Upgrades](upgrades.md)) |

Linux executables link musl statically, so one file runs on any distribution
and inside the Cloud guest, whatever C library the host has. Windows
executables link the C runtime statically, so they need no separately installed
runtime.

`xtask` itself is not released. It runs on the developer's machine and on the
release workflow's runners, where each build job builds it and the later Linux
jobs reuse what a build job of their architecture built
([Release workflow](#release-workflow)). A developer's
Linux builder of the Cloud image runs a Linux musl build of it, which the
developer's machine cross-compiles beside the native builds, for an arm64
builder with ([guest image build](../../cloud-guest-image/README.md)):

```sh
cargo zigbuild --release --locked -p xtask \
  --target aarch64-unknown-linux-musl --target-dir .cache/native-target
```

## Toolchain

`rust-toolchain.toml` pins the Rust toolchain, a nightly of a fixed date, and
lists the six targets, which rustup installs with it. Nightly is the toolchain
for development and releases alike, so that the build and the code can use
what is not yet stable:

- The compiler's and Cargo's own options are adopted when a measurement shows
  them faster or smaller, and the measurement goes beside the option. On the
  pinned nightly the toolchain alone rebuilt 10 to 25% faster after an edit
  than 1.98.1 (4 x86-64 cores, seven edit scenarios). The parallel front end
  (`-Z threads=8`) was slower there, because Cargo already keeps every core
  busy, and the Cranelift backend was no faster after an edit and does not
  catch panics, which the runner and the command services rely on
  ([Build profiles](#build-profiles)); neither is enabled.
- A crate uses an unstable language or library feature where it makes the
  code clearly simpler or faster, enabled with `#![feature]` at the crate's
  root. Each is taken up when the code that it simplifies changes, not in a
  sweep.
- The pin moves to a newer nightly as a change of its own, which passes the
  checks of [Validation](#validation) on it. The contract crates are ordinary Rust, and no build
script generates contract code or needs the frontend's tooling, so a Cargo build
needs only this toolchain and, for host builds, a C compiler for the
dependencies that compile C code. The web app's TypeScript contracts come from
a separate command, `xtask contracts`, which the frontend's scripts run through
`bun run contracts` before they need them
([Generated TypeScript](../architecture/contracts.md#generated-typescript)).

Cross builds use cargo-zigbuild with Zig for the Apple and Linux targets, and
cargo-xwin with LLVM for the Windows targets. With these tools a developer's
machine, Linux or macOS, builds every target a change needs; no target needs a
build machine of its own platform. `scripts/native/Dockerfile` pins their
versions, and the release workflow installs the same Zig and cargo-zigbuild
on its Linux runners, cargo-zigbuild as its prebuilt executable. Install the same versions on the build machine; on
macOS:

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

`cargo xtask` pins the remaining inputs: the Apple SDK version (macOS 26.5),
the Windows SDK and C runtime versions, and the minimum macOS version (13.0).
The Apple targets need an Apple SDK directory, passed with `--sdk` or
`SDKROOT`; `cargo xtask` checks its SDK metadata against the pin before
building. The pinned SDK is the one the Command Line Tools and Xcode 26.6
install, so a developer's Mac and the release workflow's macOS runner build
against the same SDK; a runner image whose SDK differs fails the check rather
than building against another one. On Windows the build checks the MSVC
toolset and Windows SDK that the runner's Visual Studio selects against the
same pins that cargo-xwin downloads from another platform. `aws-lc-sys`, the C
library of rustls's provider, compiles with cargo-xwin's `clang` from another
platform, so that build explicitly selects the MSVC driver dialect and release
optimization to match cargo-xwin's SDK flags; on Windows arm64 it compiles
with the `clang-cl` that the Visual Studio installation carries, as the crate
requires there. Both use the crate's prebuilt NASM objects for its x86-64
assembly instead of a NASM install.

## Build profiles

The workspace's `Cargo.toml` holds the two profiles, and nothing else sets
compiler options: no `RUSTFLAGS`, no per-machine configuration.

The release profile makes the executables small, because every paired device
downloads each one and every Cloud image carries them, without making a
release wait on one processor core: thin link-time optimization
(`lto = "thin"`) with Cargo's default code-generation units, optimization for
size (`opt-level = "s"`) and stripped symbols (`strip = true`). Fat
link-time optimization with one code-generation unit made the runner about a
fifth smaller (24.1 MB against 30.9 MB for x86_64 Linux, 7.9 MB against
8.7 MB compressed), but its final link runs on one core: it took about five
of a release's eleven minutes with every dependency cached, where thin takes
about a sixth of that, its optimization spread over the units. The two run
the runner's work at about the same speed. `opt-level = "z"` would save a
quarter more, but it also gives up the speed optimizations that `"s"` keeps,
and the runner's utilities run searches and sorts in process. Panics unwind in
every profile: the runner contains a utility's panic to its job
([Shell jobs](../execution/runner.md#shell-jobs)), and a resident command
service fails one invocation, not every conversation it holds; `panic =
"abort"` would end the whole process.

The development profile keeps line tables for backtraces and no other debug
information, builds dependencies without debug information, and optimizes the
few dependencies whose unoptimized code slows the tests, each with the
measurement in a comment beside it. Incremental compilation stays on for the
workspace's crates: it makes a rebuild after an edit two to three times
faster. The vendored crates are path dependencies, which Cargo would compile
incrementally too; they never change, so they build without it.

Cargo never deletes a compiled unit. Each change to a dependency, its version
or its features produces new units beside the old ones, so a target directory
grows without bound, and a build in a directory of hundreds of thousands of
stale files spends most of its time in the kernel. When a target directory
has grown large, delete it by hand: a full build of the one selection
([Validation](#validation)) takes about a minute.

## Cross builds

A developer builds on their own machine, with its own toolchain and cross
tools; the build container below is only for a machine that lacks them. The
release workflow runs the same command on each platform's runner, naming that
platform's targets ([Release workflow](#release-workflow)).

```sh
cargo xtask native build \
  --sdk /Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk
```

Without `--package` or `--target` options, `cargo xtask native build` builds
the runner and the command programs for all six targets. Repeated
`--package <crate>` options name the executables to build, including
`demi-backend` and `demi-machine-manager`. Repeated `--target <triple>` options name
the targets; without them each named executable is built for every target
[Executables and targets](#executables-and-targets) gives it. A named target
that a named executable does not run on, such as `demi-machine-manager` for a macOS
target, is refused before anything builds. `--artifacts`
selects the Cargo target directory; its default is `.cache/native-target`.

Development builds only the targets of the Hosts in use, and packages a
release of the same targets (see [Packaging](#packaging)). Building six targets
to try a change on one machine is wasted time:

```sh
cargo xtask native build \
  --sdk /Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk \
  --target aarch64-apple-darwin --target aarch64-unknown-linux-musl
```

A release build of the Linux targets prints `warning: linker stderr: ignoring
deprecated linker optimization setting '1'` once per program. Nothing in the
repository sets it: rustc passes `-O1` to a GNU-style linker at optimization
levels 2 and 3, and Zig 0.12 to 0.16 ignores the flag and says so. It is
harmless; cargo-zigbuild drops the flag in the first release after 0.23.4.

`--container <image>` runs the same build inside the image the Dockerfile
describes, for a machine without the tools. A bind-mounted checkout is slow
there, and the container and the machine do not share a Cargo target
directory, so prefer the machine's own tools. The container's target path is
`/build`, because clang-cl reads `/output` as an output flag.

```sh
docker build -t demi-native-tools -f scripts/native/Dockerfile .
cargo xtask native build \
  --container demi-native-tools --sdk /path/to/MacOSX26.5.sdk
```

## Packaging

`cargo xtask native package` turns built executables into a release directory.
It takes the same repeated `--target` options as the build and packages exactly
those targets; without them it requires every target of the executable.
`--artifacts` names the Cargo target directory the build wrote, with the
build's default.

```sh
cargo xtask native package --package demi-file \
  --artifacts .cache/native-target --output .cache/releases/demi-file-<version>
cargo xtask native package --package demi-browser \
  --artifacts .cache/native-target --output .cache/releases/demi-browser-<version>
cargo xtask native package --package demi-claude-code \
  --artifacts .cache/native-target --output .cache/releases/demi-claude-<version>
cargo xtask native package --package demi-runner \
  --artifacts .cache/native-target --output .cache/releases/runners
cargo xtask native package --package demi-backend \
  --artifacts .cache/native-target --output .cache/releases/demi-backend-<version>
```

Each executable has its own kind of release:

- **Command packages.** Each command program is released on its own. Its
  release directory holds `descriptor.json` and one subdirectory per target
  with the executable and its zstd-compressed copy, which the backend
  publishes as it is
  ([Publish packages, then source artifacts on demand](../execution/native-runtime.md#publish-packages-then-source-artifacts-on-demand)).
  Packaging keeps each compressed copy in `.cache/compressed/<sha256>`, named
  by the executable's SHA-256, and reuses it for the same executable, so
  packaging an unchanged program again compresses nothing. `xtask dev`
  compresses its development programs at zstd's fast level instead, into
  `.cache/compressed-fast`, which no release reads. The descriptor's id
  and operations are the ones the
  package's contract crate declares (`command-package-file-protocol` for `demi-file`,
  `command-package-browser-protocol` for `demi-browser`, `command-package-claude-code-protocol` for
  `demi-claude-code`), the operation list the program routes by, so a release
  cannot advertise an operation the program does not serve; its version is
  the workspace version in the release workflow and a development version
  everywhere else ([Rust executables](package-versioning.md#rust-executables)).
  A release carries only the program: software the program installs, such
  as `demi-browser`'s Chrome for Testing, is the program's own
  ([Browser distribution](../browser/browser.md#browser-distribution)).
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
    "executable": "demi-machine-manager",
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

A release carries the targets it was packaged with. The release workflow
packages all six; a developer packages the targets of the Hosts in use. The
backend loads either kind the same way, whatever its object store
([Publish a complete release](../execution/native-runtime.md#publish-a-complete-release)).
A packaged version is immutable: packaging different artifacts needs a new
workspace version, or, for a developer's release, removing its directory
before packaging it again.

The backend loads the command package releases of its
[server release](#server-release) and supplies the selected descriptors and
artifact locations to runners; publishing writes no release catalog into
application source. Before it accepts requests, the backend publishes the
command artifacts into its object store, and runners download them from there
([Backend deployment configuration](../execution/native-runtime.md#backend-deployment-configuration)).

The Cloud image embeds a Linux runner release and the command package releases.
`cargo xtask cloud-image package` assembles the image on a Linux builder of the
image's architecture: [Cloud images](../cloud/images.md) defines the image, and
the [guest image build](../../cloud-guest-image/README.md) gives the steps.

### Server release

A server release is one directory per version and Linux target, its root, in
which the backend and the machine manager find everything they serve, publish
or run, and the release's files beside it: each program the backend serves
to runners, one file per program and target. A server keeps it under
`/opt/demi/releases/` ([One release on a server](upgrades.md#one-release-on-a-server)).
For example, 0.1.3 for x86_64:

```text
/opt/demi/releases/0.1.3/
  bin/demi-backend              the target's backend
  bin/demi-machine-manager      the target's machine manager
  bin/demi-server               the server's installer and upgrader
  systemd/                      the units of the backend and the machine manager
  web/                          the built web app, with its build.json
  runners/                      the runner release's manifests
  commands/demi-file/           one command package release per directory, its descriptor
  commands/demi-browser/
  commands/demi-claude-code/
  release.json                  where the release's files are
  image/                        the Cloud image release of the target's architecture

the release's files:
  demi-runner-x86_64-unknown-linux-musl       a runner executable, per target
  demi-runner-x86_64-pc-windows-msvc.exe
  demi-file-x86_64-unknown-linux-musl.zst     a command program's compressed copy,
  demi-file-x86_64-pc-windows-msvc.exe.zst    per program and target
  ...
```

`release.json` names where the files are: for a published release, the
GitHub release that offers them; for a developer's, the directory they lie
in ([Backend deployment configuration](../execution/native-runtime.md#backend-deployment-configuration)).
The backend takes each program from there the first time a Host needs it
([Publish packages, then source artifacts on demand](../execution/native-runtime.md#publish-packages-then-source-artifacts-on-demand)),
so a server downloads the Windows programs only when a Windows laptop pairs.

The backend serves `web/`, installs runners from `runners/`, and publishes
every package under `commands/`; the machine manager imports `image/`
([Backend configuration](../backend/backend.md#configuration),
[Cloud setup](../cloud/setup.md#configuration)). The image is built from the
same release, its Linux target's programs from the release's files, so the
command artifacts it embeds are the ones the backend's catalog selects, and a
Cloud starts them from the image instead of downloading them
([Preinstalled artifacts](../execution/native-runtime.md#preinstalled-artifacts)).

`cargo xtask server-release` assembles a release from the executables
`cargo xtask native build` wrote, packaging them as the packaging above does:

```sh
cargo xtask server-release --output .cache/server/0.1.3 --files .cache/server/0.1.3-files \
  --server x86_64-unknown-linux-musl --web packages/web/dist
```

- `--output` names the root, a new directory: a root is assembled once and
  never changed in place. `--files` names the directory of the release's
  files, which may already hold this build's files, as when the build's
  other Linux root was assembled into it; a file of the same name there must
  hold the same bytes, or the assembly fails.
- `--downloads <url>` writes that URL into `release.json`, where the files
  will be published; without it, `release.json` names the `--files`
  directory.
- Repeated `--target` options name the targets of the runner and the command
  packages; without them it requires all six, as packaging does.
- `--server <triple>` puts that Linux target's backend, machine manager and
  `demi-server` in `bin/` and the services' units in `systemd/`. Without it
  the root has neither: a developer's backend runs from the Cargo target
  directory. No root carries gVisor ([gVisor runtime](#gvisor-runtime)).
- `--web <directory>` copies the built web app into `web/`. Without it the
  root has no `web/`, and the backend serves no web app, as in development,
  where Vite serves it.
- The image build adds `image/` afterwards
  ([guest image build](../../cloud-guest-image/README.md)). A root without it
  serves a backend whose machine manager runs elsewhere.

## Release workflow

`.github/workflows/release.yml` builds a release and publishes it as a GitHub
release. It runs on GitHub's standard hosted runners, which cost nothing for a
public repository, and on no machine of a developer's. One event starts it,
and nothing else does; an ordinary push builds no release:

- **A manual start on a branch**, `gh workflow run release.yml --ref
  <branch>`, releases the workspace version in `Cargo.toml` of that branch's
  newest commit. It refuses a version that already has a release, and it
  never changes the version. Publishing creates the tag `v<version>` on the
  commit it built; the npm packages' tags name their package
  ([Version selection](package-versioning.md#version-selection)), so the two
  never collide.

A release starts on a branch, never by pushing its tag, because a run reads
only the caches of its own branch and the default branch: a run started by a
tag could not read what the previous release's run cached under its tag
(see below).

The workflow runs as much at once as its jobs allow: a job waits only for the
files it uses. Each build job builds only the targets of its own platform
([Executables and targets](#executables-and-targets)), on the newest
standard runner of that platform, and builds one target's group of programs,
so that no runner compiles two targets or two groups one after the other:

| Jobs | Runner | What each builds |
| --- | --- | --- |
| Programs, per target | `macos-26` for both Apple targets, `ubuntu-26.04`, `ubuntu-26.04-arm`, `windows-2025`, `windows-11-arm` | The runner, in one job, and the three command programs, in another, for that target |
| Server, per Linux target | `ubuntu-26.04`, `ubuntu-26.04-arm` | The backend, the machine manager and `demi-server` |
| Web | `ubuntu-26.04` | `bun run build`: the published packages, then the web app that imports them |

The jobs after them each start once their inputs exist:

```text
programs (12 jobs) ─┬─▶ server release ──▶ image amd64 ─┬─▶ publish
server (2 jobs) ────┤        │                          │
web ────────────────┘        └───────────▶ image arm64 ─┘
```

- **Server release** (`ubuntu-26.04`): assembles the release of each Linux
  target, whose `release.json` names this release's GitHub location, and the
  release's files, which every target's release shares.
- **Image amd64, image arm64** (`ubuntu-26.04`, `ubuntu-26.04-arm`): builds the
  Cloud image of its architecture from that release and its files
  ([guest image build](../../cloud-guest-image/README.md)).
- **Publish** (`ubuntu-26.04`): uploads the assets below with their SHA-256
  sums.

No job compiles what another already compiled. A job of command programs
also packages them for its target, which compresses them; the server release
job reuses those compressed copies instead of compressing every target's
programs itself. The Linux runner jobs keep the `xtask` they built for
running `cargo xtask native build`, and the server release and image jobs of
the same architecture run that `xtask` instead of compiling their own. The
`xtask` a CI job builds leaves out the commands that only a developer runs,
`xtask contracts` and `xtask dev`, and with them the backend they compile.

Nor does a job compile again what its run of the previous release compiled.
Every job that compiles Rust, each build job and the web app's, keeps its
compiled dependencies in the GitHub Actions cache, one entry per job, and
starts from its entry of the previous release
(`.github/actions/rust-cache`): a release changes `Cargo.lock`, and only the
dependencies that changed compile again. The workspace's own crates are left
out, since every release changes their version and none would be reused; the
vendored crates in `vendor/`, which are path dependencies, are kept. A job
that fails keeps what it compiled too. The repository's cache holds up to
50 GB, paid beyond the free 10 GB, and keeps an entry for 90 days after its
last use, so an entry survives the time between releases. The image job
compiles nothing; its package downloads take half a minute and are not
cached.

Each step is a command of the repository that a developer runs too, a
`cargo xtask` command, `bun run build` or the image build script; only
`cargo xtask server-release --publish` differs, naming the command packages
with the workspace version itself. So the workflow only orders them and
carries files between jobs, as workflow artifacts kept for one day.

A release has these assets:

| Asset | Contents |
| --- | --- |
| `demi-<version>-server-linux-amd64.tar.zst`, `demi-<version>-server-linux-arm64.tar.zst` | The root of each architecture, without `image/`; runsc makes it about 60 MiB larger |
| `demi-<version>-image-linux-amd64.tar`, `demi-<version>-image-linux-arm64.tar` | `image/` of each architecture |
| `demi-runner-<target>.zst`, `demi-runner-<target>.exe.zst` on Windows | The release's files: each target's runner executable compressed with zstd, six files; the runner manifest's size and SHA-256 are the decompressed executable's |
| `<executable>-<target>.zst` | The release's files: each target's compressed copy of each command program, eighteen files |
| `install.sh`, `demi-server-x86_64-unknown-linux-musl`, `demi-server-aarch64-unknown-linux-musl` | The installer's bootstrap and the program it runs ([Installation](installation.md#the-bootstrap)) |
| `SHA256SUMS` | The SHA-256 of each asset above |

Unpacking a server archive and the image archive of the same architecture
into one directory gives that architecture's root as a server runs it. The
asset names and the format of `SHA256SUMS` never change: a server's
`demi-server` of an earlier release finds the next release by them
([What crosses releases](upgrades.md#what-crosses-releases)). The
image's root filesystem is already compressed, so its archive only collects
its two files. Every asset stays below GitHub's limit of 2 GiB per file; an
image's root filesystem is about 700 MiB, and a server archive holds only the
backend, the manager, the web app and the manifests. A paired device does
not download from the release: it installs its runner from its backend,
which takes it from the release the first time.

The macOS and Windows executables are not signed or notarized, and a release
never will be. Apple's linker gives each macOS executable the ad-hoc signature
that arm64 requires. The installers download executables with `curl` or
PowerShell rather than a browser, so the downloads carry no mark that would
make the system ask before running them.

## Chrome for Testing

Each Demi release pins one Chrome for Testing version, which `demi-browser`
installs when the agent runs `demi browser install`
([Browser distribution](../browser/browser.md#browser-distribution)).
`cargo xtask browser-release` pins the version it is given:

```sh
cargo xtask browser-release 153.0.8010.36
```

It reads that version's official download metadata and, for each platform
Demi supports, downloads the `chrome` archive from Chrome for Testing's
download host through the artifact library, measures its size and SHA-256, and
checks that it holds the executable the record names. It then writes the
release record, `crates/command-package-browser-protocol/src/release/chrome.json`,
which the program compiles in; commit it with the change that adopts the
version. Chrome for Testing publishes no Windows arm64 build, so the record
carries five of the six targets. The downloads are not kept: nothing in a
release carries Chrome.

Beside it, `crates/command-package-browser-protocol/src/release/linux.json` lists
what Chrome needs on Linux that Demi does not install: each shared library
Chrome loads that Ubuntu does not ship by default, with the Ubuntu package
that provides it, and the font packages that let pages show emoji and
Chinese, Japanese and Korean text. `demi browser install` names from it what
a Host lacks ([Installation](../browser/browser.md#installation)). The list is
kept by hand: adopting a new version checks it on an Ubuntu Host of each
architecture by installing Chrome on a minimal system, adding what the list
names, and starting it.

## gVisor runtime

No release carries gVisor. The machine manager is built against the one
`runsc` version that `crates/machine-manager/runtime/release.json` pins,
and refuses any other; `demi-server` fetches that version onto a server
before a release that pins it runs, as a setup or an upgrade's preparation
([One release on a server](upgrades.md#one-release-on-a-server)). It is
built from the same workspace, so it knows the pin, and it fetches the
archive for the server's architecture through the artifact library, checks
it against the pinned SHA-512, and unpacks into
`/opt/demi/gvisor/<version>/` only what the manager runs: `runsc` and the
sidecar programs `runsc` starts from `gvisor-bin/` beside it,
`gvisor_sentry`, `gvisor-sentry-prewarmer` and `runsc-fd-parking`. The
distribution's other programs, the containerd shim, the metric server and
the memory checkpoint gofer, stay out; the manager uses none of them, and
leaving them halves the size. The manager runs `runsc` with
`--sidecar-usage-policy=STRICT`, so a missing sidecar fails rather than
being downloaded by `runsc` itself or replaced by its deprecated embedded
copy ([Isolation and joining](../cloud/managed-hosts.md#isolation-and-joining)).

The amd64 archive is upstream's release. The arm64 archive carries Demi's
patch, so it is built from the pinned source once per pin, not in every
release: `.github/workflows/runtime.yml`, which pushing the tag
`runsc-<arm64Version>` starts, runs
`crates/machine-manager/scripts/build-runsc-arm64.sh` and the regression
probe on `ubuntu-26.04-arm` and publishes the archive as the GitHub release
of that tag. The change that adopts a new pin records that archive's
SHA-512 in the manifest beside the version.

A developer's Linux host gets the pinned version the same way, with
`demi-server runtime`, which fetches it into `/opt/demi/gvisor/<version>/`
alone.

## Validation

[Testing](testing.md) says what a test must be; this section says how the tests run.

The [release workflow](#release-workflow) builds and publishes; it runs no
check. Developers run the checks on their machines, and release acceptance
runs the shared Rust suite on each platform that ships a feature.

Every Rust command but the Chrome suite's selects the same thing: the whole
workspace with the runner's `test-fixtures` feature, which turns on the
runner libraries' test support and so builds the fixture programs the tests
start. Cargo unifies features over what one command
selects, so a command that selected one crate or other features would build
its own copy of every shared dependency.

| Command | What it runs |
| --- | --- |
| `cargo check --workspace --all-targets --features demi-runner/test-fixtures` | The type check of every crate, test and example |
| `cargo test --workspace --features demi-runner/test-fixtures` | The Rust tests, the crate boundary check among them ([Boundary checks](../architecture/crates-and-packages.md#boundary-checks)); `--test <name>` runs one test target |
| `DEMI_TEST_CHROME=<chrome> cargo test --workspace --features demi-runner/test-fixtures --test browser -- --include-ignored --test-threads=1` | The tests that start Chrome, one at a time, with the executable of the pinned Chrome for Testing release, unpacked; they run as an ordinary user, since Chrome refuses root on Linux with its sandbox |
| `DEMI_TEST_CHROME=<chrome> cargo test --workspace --features demi-runner/test-fixtures --test backend -- --ignored real_browser` | The browser suite's scenario through the backend and a paired device's runner ([Browser suite](scenarios.md#browser-suite)), as an ordinary user with the same executable |
| `DEMI_TEST_CLAUDE_CODE=<claude> SSL_CERT_FILE=$PWD/crates/backend/tests/backend/claude_code/distribution-ca.pem cargo test --workspace --features demi-runner/test-fixtures --test backend -- --ignored claude_code` | The Claude Code suite, with the executable of the vendor's CLI and the CA of the suite's local distribution |
| `bun run test` | The TypeScript tests, the package boundary check among them ([Boundary checks](../architecture/crates-and-packages.md#boundary-checks)), and the test of the capture extension's JavaScript, which sits beside the extension in `command-package-browser-chrome`; it first builds the programs the tests start, with the same selection |
| `sudo bash crates/machine-manager/scripts/cloud-suite.sh --release <root> --work <directory>` | The Cloud suite on Linux, as root, against a machine manager with its resource limits off that the script starts in a stand-in execution host; `--programs <directory>` for programs built elsewhere, as in Lima ([Cloud suite](scenarios.md#cloud-suite)) |

A test that starts another program, such as a runner or `demi-file`,
starts the one Cargo built into the target directory the test runs from
(`command_protocol::testing::built_program`); it builds nothing itself.
`cargo test` of the whole selection builds every program first: Cargo builds
a package's executables for that package's integration tests, so every crate
whose program a test starts has an integration test binary. One test target
builds only what it links, so before running one alone that
starts another crate's program, build the selection with
`cargo build --workspace --all-targets --features demi-runner/test-fixtures`.
The TypeScript tests take the programs from `DEMI_TEST_PROGRAMS`, which
`bun run test` sets to that directory. An ordinary test run starts no Chrome;
release acceptance runs the Chrome tests. `DEMI_TEST_CHROME` names the
executable of an installation of the pinned release that the user running the
tests can read in full, such as the one a runner installs in its artifact
cache. A test that drives the browser program answers its request for Chrome
with that executable, as the runner would; the scenario through the backend
installs that release into its runner's artifact cache, as the runner
installs a download, with hard links where the system allows them
([Install artifacts](../execution/native-runtime.md#install-artifacts)):
no test downloads Chrome or needs the home. The live view tests decode the H.264
pictures the view streams with WebCodecs in the Chrome under test, as the page
does. On Linux the Chrome tests need an ordinary user: Chrome for Testing
refuses to start as root with its sandbox, which Demi keeps
([Native driver](../browser/browser.md#native-driver)). An ordinary test run
also skips the Cloud suite, which needs a machine manager, a Cloud image, and
root. The machine manager builds only for Linux, and on Linux the one
selection builds and runs its tests
([Verification](../cloud/managed-hosts.md#verification)); on a Mac they run in
a Lima VM ([Develop on a Mac with Lima](../guides/mac-development.md#machine-manager-tests)). The tests that need
root are ignored in an ordinary run; as root, the manager's unit test
executable with `--include-ignored` runs them, each in mount and network
namespaces of its own:

```sh
cargo test --workspace --features demi-runner/test-fixtures --no-run
sudo target/debug/build/demi-machine-manager/<hash>/out/demi_machine_manager-<hash> --include-ignored
```

The Claude Code suite runs as an ordinary user, as the Cloud's runner does:
the CLI refuses the provider's permission mode as root. Its local
distribution serves HTTPS, since `demi.claude-code` downloads nothing else, and
`SSL_CERT_FILE` names the CA the suite carries for it, for the test process and
the runners it starts only. The backend in the test process and the Cloud's
`demi.claude-code` trust that CA because the one selection builds reqwest with the
machine's roots, which the variable replaces; a selection without them fails
the suite's install with an unknown issuer. The CA's key was discarded, and
its certificate and the distribution's expire on 2126-09-03.

[Scenarios](scenarios.md) defines the suites that run the whole backend.

Release acceptance also checks the artifacts and installers on each platform:

- A Linux executable with an ELF interpreter or a shared-library dependency
  fails.
- Windows exercises the PowerShell installer; Linux and macOS exercise the
  shell installer. These checks include registration separation, release
  reuse, and draining upgrades.
- Each installer and publication fixture runs three times per platform to
  exercise repeated process startup and teardown.

`crates/command-sdk/examples/benchmark.rs` is a standalone synthetic
service and client. Build it with
`cargo build --release -p demi-command-sdk --example benchmark`, then run
`target/release/examples/benchmark` to measure the machine and build it runs on.
It never calls a model.

## Cloud image refresh

The Cloud runs the Linux target that matches its execution host. Build and
package that target together with the paired-device target used for acceptance.
The [Cloud image contract](../cloud/images.md#acceptance-and-local-refresh) owns
embedding, manager restart, and checking the identities of the running
artifacts; rebuilding a native release alone does not refresh a Cloud, whose
runner and embedded programs come from the image the manager is configured
with. A change to the machine manager itself is built for its host's Linux
target and reaches the host in a release ([Upgrades](upgrades.md)).
