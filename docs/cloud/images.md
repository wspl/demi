# Build and release Cloud images

A Cloud base supplies the files that gVisor runs: Ubuntu, development tools,
init, the native runner and the command programs. It holds no browser and no
guest kernel or virtual hardware configuration. The manager combines those
files with each device's persistent storage.

What a base holds and why is in `cloud-guest-image/rootfs/packages.txt` and
`cloud-guest-image/rootfs/build.sh`; its manifest and what the manager relies
on in a base are in `crates/machine-manager-protocol/src/image.rs`; how the
manager imports one is in `crates/machine-manager/src/storage/base.rs`. [Cloud
setup](setup.md) describes deployment, and the
[build instructions](../../cloud-guest-image/README.md) assemble and publish a
release on a matching Linux builder.

## Building

An image release is the `image/` directory of a server release
(`crates/xtask/src/server_release.rs`), built from that release's runner and
command packages, whose programs for the image's target it takes from the
release's files. A published release carries it as an asset of its own per
architecture (`.github/workflows/release.yml`).

Build amd64 and arm64 separately. Native executable targets are
`x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, respectively.
Assemble the Linux filesystem on a Linux builder of the image's architecture:
for a published release, the release workflow's `ubuntu-26.04` and
`ubuntu-26.04-arm` runners, which give a job root through passwordless `sudo`;
for development, a Linux machine or VM of the developer's, with the targets of
the Hosts in use packaged by `bun xtask native build` on the developer's
machine. Do not confuse cross-compiling a Rust executable with validating the
complete image on another architecture.

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
checkpoint. Rebuilding a native release alone does not refresh a Cloud, whose
runner and embedded programs come from the image the manager is configured with.

Image acceptance is separate from the six-target native release. The latter
also covers macOS and Windows paired devices; the Cloud base is Linux only.
