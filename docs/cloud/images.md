# Build and release Cloud images

A Cloud base supplies the files that gVisor runs: Ubuntu, development tools,
init, the native runner and the command programs. It holds no browser: the
agent installs one with `demi browser install` when it needs it
([Browser distribution](../browser/browser.md#browser-distribution)). It contains no guest kernel or virtual
hardware configuration. The manager combines those files with each device's
persistent storage, as defined in [Managed hosts](managed-hosts.md#images).

[Cloud setup](setup.md) describes deployment. The
[build instructions](../../cloud-guest-image/README.md) assemble and publish
this format on a matching Linux builder.

## Release artifacts

`cloud-guest-image` holds the Linux root filesystem build. Each architecture
produces:

| Artifact | Contents |
| --- | --- |
| `rootfs.tar.zst` | A complete root tree with numeric ownership, modes, symlinks, hardlinks, and extended attributes preserved. |
| `manifest.json` | Versioned schema, Linux architecture, archive size and SHA-256, pinned build inputs, and the embedded executable hashes. |

The manifest uses `formatVersion: 1`, `os: linux`, and
`architecture: amd64 | arm64`. It records the exact Ubuntu release, package
inventory, and runner/native release descriptors. Its `rootfs` entry names the archive and its byte size and SHA-256.
Its runner release must name the embedded runner executable, and each command
package release must have its artifact for the image's target embedded under
that artifact's content-addressed path. The SHA-256 of the exact manifest file
bytes is `baseVersion`; one hash names one immutable build. The manifest's type
is defined once, in the `machine-manager-protocol` crate: the packaging command
validates a manifest with it before publishing, and the manager validates a
release with it before importing.

The archive is a prepared OCI root filesystem, not an OCI runtime configuration.
The manager generates the per-boot OCI bundle. Image content cannot select
runtime flags, privileged mounts, cgroups, network exceptions, or credentials.
No Docker daemon, image pull, archive extraction, or compilation is part of an
ordinary wake. Transport can copy the release directory to an execution host;
the manager consumes local verified artifacts. An image release is the
`image/` directory of a [server release](../delivery/builds-and-releases.md#server-release),
built from that release's runner and command packages, whose programs for
the image's target it takes from the release's files, and a published
release carries it as an asset of its own per architecture
([Release workflow](../delivery/builds-and-releases.md#release-workflow)).

Build amd64 and arm64 separately. Native executable targets are
`x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, respectively.
Assemble the Linux filesystem on a Linux builder of the image's architecture:
for a published release, the release workflow's `ubuntu-26.04` and
`ubuntu-26.04-arm` runners, which give a job root through passwordless `sudo`;
for development, a Linux machine or VM of the developer's, with the targets of
the Hosts in use packaged by the
[native cross tools](../delivery/builds-and-releases.md) on the developer's
machine. Do not confuse cross-compiling a Rust executable with validating the
complete image on another architecture.

## Build pipeline

A build runs as root on a Linux builder of the target architecture, in two
stages:

1. `cloud-guest-image/rootfs/build.sh` creates the Ubuntu tree from
   Ubuntu's official container root filesystem, the archive of the serial
   that `cloud-guest-image/rootfs/ubuntu.json` pins, checked against the
   SHA-256 pinned there for the builder's architecture. Inside a chroot, it
   brings the tree's packages up to date, installs the listed packages and
   `tini`, removes the container image's own user `ubuntu`, which holds the
   UID and GID 1000 that are `demi`'s, creates the `demi` user and its sudo
   rule, applies the file
   overlay, and removes package caches, runtime state, and machine identity.
   These steps drive a package manager inside the tree, so a shell script
   runs them.
2. The packaging command, `bun xtask cloud-image package`, completes the
   release. It installs the runner and its `demi` alias from the verified
   runner release and the command packages' executables from their verified
   releases. It reads the package inventory from the tree's dpkg database without
   running a program of the image: the packages dpkg records as installed,
   with their versions. A package whose installation did not finish fails the
   build; one removed with only its configuration left is not installed. The
   manifest's Ubuntu release is the tree's own, `VERSION_ID` in its os-release
   file. The command writes the archive with GNU tar, which keeps numeric
   ownership, ACLs, and extended attributes; validates and writes the
   manifest; publishes the release directory atomically; and prints the
   `baseVersion`.

On the builder, the script runs a Linux build of `xtask`: one that the
developer's machine cross-compiles for the builder's architecture with its own
cross tools, as it does the runner
([Builds and releases](../delivery/builds-and-releases.md)), or, in the
release workflow, one the runner builds for itself.
The image has the architecture that build runs on. On a Linux builder, the
`xtask` that the workspace's one Cargo selection builds into `target/debug`
also works; it downloads nothing, since the release's files are local. The
script's downloads, the container image and the packages, go through the
builder's HTTPS proxy, and the
[build instructions](../../cloud-guest-image/README.md) give the options for
a proxy that re-signs TLS.

## Root filesystem contents

The base holds what Demi needs in every Cloud and what nearly every task
needs, and nothing else; every Cloud shares it, and every megabyte in it is
downloaded and stored on each server. It starts from Ubuntu's official
container image, which is minimized: its dpkg configuration leaves out man
pages and translations, for the packages installed later too. On it the
build installs only what
[packages.txt](../../cloud-guest-image/rootfs/packages.txt) lists: `sudo`, for
the agent to install the rest; `ca-certificates`, for HTTPS; and `git`, which
nearly every task uses. The shell, its commands, and the tools Demi's own
commands use are Demi's executables, not the image's. Compilers, language
runtimes such as Node.js and Python, and tools such as `gh` are installed on
demand, by the agent or the user: with `apt` into the system layer, which a
system reset empties, or with a version manager into the home, which every
reset keeps. Nothing is there for the browser either: `demi browser install`
brings Chrome with the libraries and fonts it needs into the home, which a
system reset keeps, as on any Linux Host
([Browser distribution](../browser/browser.md#browser-distribution)).
The locale is `C.UTF-8`, the one the container image carries. The manifest
records the resolved package versions rather than a second version list in
documentation; with Demi's programs, the base is about 260 MB unpacked and
80 MB compressed.

The container image is checked against a digest pinned in the repository: a
digest fetched beside the archive would prove only that the download arrived
intact, not that it is the file that was reviewed.
`cloud-guest-image/rootfs/ubuntu.json` pins the serial and, for each
architecture, the archive's URL on `partner-images.canonical.com`, its size,
and its SHA-256. The build brings the packages up to date from the archive's
mirrors, so an image carries the security updates published by its build,
whatever the serial's age; a pin moves to a newer serial when Canonical
removes the old one, and for a newer Ubuntu release.

The image supplies `demi` UID/GID 1000, passwordless sudo and a minimal init
(`tini`). Everything of Demi's own lies under `/opt/demi`, which the machine
manager replaces with the configured image's at every boot, whatever image a
Cloud is pinned to
([Demi's programs in a Cloud](managed-hosts.md#demis-programs-in-a-cloud)):

- `/opt/demi/bin/demi-runner`, and `/opt/demi/bin/demi`, the command alias
  the native runtime expects. `/usr/bin/demi-runner` and `/usr/bin/demi` are
  symbolic links to them.
- Each embedded command package's executable, at its content-addressed path
  `/opt/demi/artifacts/<sha256>/<executable>`, the one file in the directory
  its SHA-256 names. The runner starts command services from these copies
  instead of downloading the artifacts, after checking each against the
  backend's pinned descriptor
  ([Preinstalled artifacts](../execution/native-runtime.md#preinstalled-artifacts)).

The image also makes the runner's two directories on the system layer,
private to `demi`: `/var/lib/demi` for the job directories and the
temporary directory, and `/var/log/demi` for the Host log
([Images](managed-hosts.md#images)). The
embedded artifacts' identities must match the backend's selected releases:
the runner downloads a selected artifact that the image does not hold, on the
first command after every wake and reset. Since the image and the backend
come from one server release, they match.

What the manager relies on in a base, the `demi` user, init, the runner's
directories and `/opt/demi` as the place of Demi's programs, is the base's
format, which the manifest's `formatVersion` names. A release that changes
any of it changes the format.

The image contains mount points for `/home`, `/run`, `/tmp`, `/dev`, `/proc`,
and `/dev/shm`. It does not mount them or configure routes at runtime. Service
startup is disabled during package installation; no systemd boot or
distribution service manager is required in the sandbox. The image entry
process and home initialization are defined only in
[container initialization](managed-hosts.md#container-initialization).

Home-based version managers and runtimes are installed on demand. The base does
not preinstall nvm, rustup, Go, a JDK, or Docker. `/etc/skel` keeps installer
setup outside interactive-only shell guards, so subsequent login jobs can find
user tools. User changes under home belong to the home volume.

Builds contain no device credentials, host checkout paths, package-registry
credentials, machine ids copied from a builder, or temporary package caches.
The build permits setuid files needed by the selected sandbox profile, including
sudo; extraction must preserve them. They execute only inside gVisor.

## Import and publication

Build into a staging directory, verify every embedded artifact against its
release descriptor, create the archive and manifest, and publish the directory
atomically. The manager imports its configured release once, at startup, before
it serves requests; a failed import stops startup with its diagnostic. An
[upgrade](../delivery/upgrades.md#prepare) imports the next release's image
earlier: the next release's manager, run with `--import`, imports its own
release's image beside the running manager and exits, so the start finds it
imported. It takes none of the running manager's locks, since a base is an
immutable directory named by its `baseVersion` and published atomically, and
a running manager reads no base it was not configured with. Import
rejects unknown schemas, mismatched hashes or sizes, missing executable
entries, and an architecture that differs from its Linux host. A base already
imported under the same `baseVersion` must hold the same manifest bytes.

Block devices, sockets, and runtime state are not permitted archive entries.
Import checks the archive before extracting it. It reads every entry with a
streaming tar reader and refuses any entry that is not a regular file,
directory, symbolic link, or hard link, and any path or hard-link target that is
absolute or contains `..`. `bsdtar` then extracts the archive into the staging
tree, preserving the required metadata; it never follows an archive path into
the host filesystem. Each executable the manifest lists is opened beneath the
extracted root without following a link out of it, and must be a regular file
whose size and hash match. Import strips no unexpected input to make a corrupt
release appear valid: reject it. Sync the extracted tree and manifest before
publishing the immutable base directory. The manager never executes image
content on the host during import.

The base directory is writable only during staging, then exposed read-only as
OverlayFS's lower layer. Ordinary wake uses the base named by the stored
generation; changing the configured release does not rewrite that reference,
and only `/opt/demi` comes from the configured base
([Demi's programs in a Cloud](managed-hosts.md#demis-programs-in-a-cloud)).
Collection and reset pinning follow
[Base retention](managed-hosts.md#base-retention).

## Acceptance and local refresh

Verify the full image on each supported architecture with the shipped runsc
profile: init reaps orphans, jobs use UID 1000, sudo works, package installation
works, and Chrome, once the agent has installed it and
its libraries, retains its sandbox. Test shell/native
commands and the conversation browser through the real managed runner
connection.

When a runner or embedded command package changes, build the Cloud target,
rebuild the image and restart the local manager with it; the Cloud's next wake
runs the new programs, with no reset. Check the running runner's executable
hash and command package identity after that wake and again after
hibernate/wake and after a reset. After each, the first native command starts its service from the
image's embedded executable, without a download. Do not hash PID 1
as a proxy for the runner: PID 1 is init. Exercise the paired device in the same
checkpoint.

Image acceptance is separate from the six-target native release. The latter
also covers macOS and Windows paired devices; the Cloud base is Linux only.
