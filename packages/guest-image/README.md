# Cloud image build

A Cloud image release is an architecture-specific Linux root archive and its
manifest for gVisor. [Cloud images](../../docs/cloud/images.md) defines the
format, the build pipeline, import, and acceptance;
[Cloud setup](../../docs/cloud/setup.md) describes deployment.

A build runs as root on a Linux builder of the image's architecture, such as
the `demi-machines` Lima VM on a Mac. `rootfs/build.sh` makes the Ubuntu tree,
then runs `xtask cloud-image package`, which embeds the runner release, the
command packages, Chrome for Testing, and uv, and publishes the release.

First, on the developer's machine, build and package the image's target with
the [native cross tools](../../docs/delivery/builds-and-releases.md), and build
`xtask` for the builder. For an arm64 image (use `x86_64-unknown-linux-musl`
for amd64):

```sh
cargo xtask native build --target aarch64-unknown-linux-musl
cargo xtask native package --package demi-runner \
  --target aarch64-unknown-linux-musl --output .cache/releases/runners
cargo xtask native package --package demi-commands \
  --target aarch64-unknown-linux-musl --output .cache/releases/demi-builtin-<build>
cargo xtask native package --package demi-claude \
  --target aarch64-unknown-linux-musl --output .cache/releases/demi-claude-<build>
cargo zigbuild --release --locked -p xtask \
  --target aarch64-unknown-linux-musl --target-dir .cache/native-target
```

Then, from the repository root on the builder, with debootstrap, GNU tar, and
util-linux installed:

```sh
sudo bash packages/guest-image/rootfs/build.sh \
  --xtask .cache/native-target/aarch64-unknown-linux-musl/release/xtask \
  --runners .cache/releases/runners \
  --package .cache/releases/demi-builtin-<build> \
  --package .cache/releases/demi-claude-<build> \
  --output /opt/demi-cloud/releases/<build>
```

In Lima, prefix the command with `limactl shell demi-machines --`: the VM sees
the Mac's home directory at the same path.

`--output` names a new directory on a Linux filesystem: a release is
immutable, so every build publishes a new one. The script builds the tree in
`/var/tmp/demi-cloud-root`, or in the directory `--work` names, and removes it
after a successful build. `--mirror` names an Ubuntu mirror other than the
official one. The last line the build prints is the release's base version.
The build neither starts nor resets a Cloud device; see
[Acceptance and local refresh](../../docs/cloud/images.md#acceptance-and-local-refresh).

`rootfs/uv.json` pins uv: its version and, for each architecture, the
archive's URL, size, SHA-256, and executables. To change the pin, download each
archive, check its SHA-256 against the uv release's published digest, record
it, and rebuild `xtask`, which carries the pin it was built with.
