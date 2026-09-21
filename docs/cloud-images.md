# Build and release Cloud images

A Cloud base supplies the files that gVisor runs: Ubuntu, development tools,
Chrome, init, and the native runner. It contains no guest kernel or virtual
hardware configuration. The manager combines those files with each device's
persistent storage, as defined in [Managed hosts](demi-next/managed-hosts.md#images).

[Cloud setup](managed-hosts-setup.md) describes deployment. The
[build instructions](../packages/guest-image/README.md) assemble and publish this
format on a matching Linux builder.

## Release artifacts

`packages/guest-image` owns the Linux root filesystem build. The directory name
is retained; it does not imply a VM. Each architecture produces:

| Artifact | Contents |
| --- | --- |
| `rootfs.tar.zst` | A complete root tree with numeric ownership, modes, symlinks, hardlinks, and extended attributes preserved. |
| `manifest.json` | Versioned schema, Linux architecture, archive size and SHA-256, pinned build inputs, and the embedded executable hashes. |

The manifest uses `formatVersion: 1`, `os: linux`, and `architecture: amd64 | arm64`.
It records the exact Ubuntu base, package inventory, standalone-tool versions and
hashes, and runner/native release descriptors. Its `rootfs` entry names the archive
and its byte size and SHA-256. The SHA-256 of the exact manifest file bytes is
`baseVersion`; one hash names one immutable build. Schema ownership belongs to
the machines package, which validates a release before importing it.

The archive is a prepared OCI root filesystem, not an OCI runtime configuration.
The manager generates the per-boot OCI bundle. Image content cannot select
runtime flags, privileged mounts, cgroups, network exceptions, or credentials.
No Docker daemon, image pull, archive extraction, or compilation is part of an
ordinary wake. Transport can copy the release directory to an execution host;
the manager consumes local verified artifacts.

Build amd64 and arm64 separately. Native executable targets are
`x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, respectively. Use
[native cross tools](native-builds.md) on the developer's machine and package only
the targets of the Hosts in use. Assemble the Linux filesystem on a matching
Linux builder, including Lima for Mac development. Do not confuse cross-compiling
a Rust executable with validating the complete image on another architecture.

## Root filesystem contents

The base is Ubuntu 26.04. Its system package inventory has one source:
[packages.txt](../packages/guest-image/rootfs/packages.txt). The build also installs
standalone `uv`, pinned Chrome for Testing using the browser package's existing
installer, and the shipped native artifacts. It records resolved versions and
hashes rather than maintaining a second version list in documentation.

The image supplies `demi` UID/GID 1000, passwordless sudo, a minimal init (`tini`),
and `/usr/bin/demi-runner`, with the `demi` command alias expected by the native
runtime. Embedded command packages use their content-addressed artifact paths
and descriptors. Their identities must match the backend's selected releases;
rebuilding the runner alone does not refresh command binaries.

The image contains mount points for `/home`, `/run`, `/tmp`, `/dev`, `/proc`, and
`/dev/shm`. It does not mount them or configure routes at runtime. Service startup
is disabled during package installation; no systemd boot or distribution service
manager is required in the sandbox. The image entry process and home initialization
are defined only in [container initialization](demi-next/managed-hosts.md#container-initialization).

Home-based version managers and runtimes are installed on demand. The base does
not preinstall nvm, rustup, Bun, Go, a JDK, or Docker. `/etc/skel` keeps installer
setup outside interactive-only shell guards, so subsequent login jobs can find
user tools. User changes under home belong to the home volume.

Builds contain no device credentials, host checkout paths, package-registry
credentials, machine ids copied from a builder, or temporary package caches.
The build permits setuid files needed by the selected sandbox profile, including
sudo; extraction must preserve them. They execute only inside gVisor.

## Import and publication

Build into a staging directory, verify every embedded artifact against its release
descriptor, create the archive and manifest, and publish the directory atomically.
A manager rejects unknown schemas, mismatched hashes or sizes, missing executable
entries, and an architecture that differs from its Linux host.

Extraction uses a maintained archive implementation that preserves the required
metadata and confines entries, symlinks, and hardlinks to the staging tree. It
must not follow an archive path into the host filesystem. Import strips no
unexpected input to make a corrupt release appear valid: reject it. Block devices,
sockets, and runtime state are not permitted archive entries. Sync the extracted
tree and manifest before publishing the immutable base directory. The manager
never executes image content on the host during import.

The base directory is writable only during staging, then exposed read-only as
OverlayFS's lower layer. Ordinary wake uses the base named by the stored generation;
changing the configured release does not rewrite that reference. Collection and
reset pinning follow [generation storage](demi-next/managed-hosts.md#save-a-generation).

## Acceptance and local refresh

Verify the full image on each supported architecture with the shipped runsc
profile: init reaps orphans, jobs use UID 1000, sudo works, package installation
and standalone tools work, and Chrome retains its sandbox. Test shell/native
commands and the browser through the real managed runner connection.

When a runner or embedded native package changes, build the Cloud target, rebuild
the image, restart the local manager, and reset local Cloud to that base before
acceptance. Check the running runner's executable hash and native package identity
after initial start and again after hibernate/wake. Do not hash PID 1 as a proxy
for the runner: PID 1 is init. Exercise the paired device in the same checkpoint.

Image acceptance is separate from the six-target native release. The latter also
covers macOS and Windows paired devices; the Cloud base is Linux only.
