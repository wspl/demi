# Build and release Cloud images

A Cloud base supplies the files that gVisor runs: Ubuntu, development tools,
Chrome, init, and the native runner. It contains no guest kernel or virtual
hardware configuration. The manager combines those files with each device's
persistent storage, as defined in [Managed hosts](managed-hosts.md#images).

[Cloud setup](setup.md) describes deployment. The
[build instructions](../../packages/guest-image/README.md) assemble and publish
this format on a matching Linux builder.

## Release artifacts

`packages/guest-image` holds the Linux root filesystem build. Each architecture
produces:

| Artifact | Contents |
| --- | --- |
| `rootfs.tar.zst` | A complete root tree with numeric ownership, modes, symlinks, hardlinks, and extended attributes preserved. |
| `manifest.json` | Versioned schema, Linux architecture, archive size and SHA-256, pinned build inputs, and the embedded executable hashes. |

The manifest uses `formatVersion: 1`, `os: linux`, and
`architecture: amd64 | arm64`. It records the exact Ubuntu release, package
inventory, standalone-tool versions and hashes, and runner/native release
descriptors. Its `rootfs` entry names the archive and its byte size and SHA-256.
Its runner release must name the embedded runner executable, and each command
package release must have its artifact for the image's target embedded under
that artifact's content-addressed path. The SHA-256 of the exact manifest file
bytes is `baseVersion`; one hash names one immutable build. The manifest's type
is defined once, in the `machines-protocol` crate: the packaging command
validates a manifest with it before publishing, and the manager validates a
release with it before importing.

The archive is a prepared OCI root filesystem, not an OCI runtime configuration.
The manager generates the per-boot OCI bundle. Image content cannot select
runtime flags, privileged mounts, cgroups, network exceptions, or credentials.
No Docker daemon, image pull, archive extraction, or compilation is part of an
ordinary wake. Transport can copy the release directory to an execution host;
the manager consumes local verified artifacts.

Build amd64 and arm64 separately. Native executable targets are
`x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, respectively. Use
[native cross tools](../delivery/builds-and-releases.md) on the developer's
machine and package only the targets of the Hosts in use. Assemble the Linux
filesystem on a matching Linux builder, including Lima for Mac development. Do
not confuse cross-compiling a Rust executable with validating the complete
image on another architecture.

## Build pipeline

A build runs as root on a Linux builder of the target architecture, in two
stages:

1. `packages/guest-image/rootfs/build.sh` creates the Ubuntu tree with
   debootstrap. Inside a chroot, it installs the listed packages and `tini`,
   creates the `demi` user and its sudo rule, applies the file overlay, and
   removes package caches, runtime state, and machine identity. These steps
   drive a package manager inside the tree, so a shell script runs them.
2. The packaging command, `cargo xtask cloud-image package`, completes the
   release. It installs the runner and its `demi` alias from the verified
   runner release, the command packages from their verified releases, Chrome,
   and uv. It reads the package inventory from the tree's dpkg database without
   running a program of the image: the packages dpkg records as installed,
   with their versions. A package whose installation did not finish fails the
   build; one removed with only its configuration left is not installed. The
   manifest's Ubuntu release is the tree's own, `VERSION_ID` in its os-release
   file. The command writes the archive with GNU tar, which keeps numeric
   ownership, ACLs, and extended attributes; validates and writes the
   manifest; publishes the release directory atomically; and prints the
   `baseVersion`.

On the builder, the script runs a Linux build of `xtask` that the developer's
machine cross-compiles for the builder's architecture with its own cross tools,
as it does the runner ([Builds and releases](../delivery/builds-and-releases.md)).
The image has the architecture that build runs on.

## Root filesystem contents

The base is the Ubuntu release that the build script pins, and the manifest
records it. Its system package inventory has one source:
[packages.txt](../../packages/guest-image/rootfs/packages.txt). The build also
installs standalone `uv`, pinned Chrome for Testing, and the shipped native
artifacts. It records resolved versions and hashes rather than maintaining a
second version list in documentation.

Chrome comes from the browser release pinned in the repository and is installed
by the same installer that paired devices use, in the `artifact` crate, so one
implementation downloads, verifies, and unpacks it everywhere. uv is checked
against a digest pinned in the repository: a digest fetched from the same
release as uv would prove only that the download arrived intact, not that it is
the file that was reviewed. `packages/guest-image/rootfs/uv.json` pins its
version and, for each architecture, the archive's URL, size, and SHA-256.
Artifact downloads follow no redirect, and uv's GitHub release URLs redirect
to short-lived storage URLs, so the pin names the same files on Astral's
release host, `releases.astral.sh`, which serves them directly.

The image supplies `demi` UID/GID 1000, passwordless sudo, a minimal init
(`tini`), and `/usr/bin/demi-runner`, with the `demi` command alias expected by
the native runtime. Embedded command packages use their content-addressed
artifact paths and descriptors. Their identities must match the backend's
selected releases; rebuilding the runner alone does not refresh command
binaries.

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
it serves requests; a failed import stops startup with its diagnostic. It
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
generation; changing the configured release does not rewrite that reference.
Collection and reset pinning follow
[generation storage](managed-hosts.md#save-a-generation).

## Acceptance and local refresh

Verify the full image on each supported architecture with the shipped runsc
profile: init reaps orphans, jobs use UID 1000, sudo works, package installation
and standalone tools work, and Chrome retains its sandbox. Test shell/native
commands and the browser through the real managed runner connection.

When a runner or embedded native package changes, build the Cloud target,
rebuild the image, restart the local manager, and reset local Cloud to that base
before acceptance. Check the running runner's executable hash and native package
identity after initial start and again after hibernate/wake. Do not hash PID 1
as a proxy for the runner: PID 1 is init. Exercise the paired device in the same
checkpoint.

Image acceptance is separate from the six-target native release. The latter
also covers macOS and Windows paired devices; the Cloud base is Linux only.
