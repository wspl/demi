# Cloud image build

A Cloud image release is an architecture-specific Linux root archive and its
manifest for gVisor. [Cloud images](../docs/cloud/images.md) defines the
format, the build pipeline, import, and acceptance;
[Cloud setup](../docs/cloud/setup.md) describes deployment.

A build runs as root on a Linux builder of the image's architecture.
`rootfs/build.sh` makes the Ubuntu tree from Ubuntu's container image, then
runs `xtask cloud-image package`, which embeds the runner release and the
command packages' executables, and publishes the release. The image goes into the `image/` directory of
the [server release](../docs/delivery/builds-and-releases.md#server-release)
whose runner and command package releases it embeds. The release workflow
builds the published images this way on its `ubuntu-26.04` and
`ubuntu-26.04-arm` runners
([Release workflow](../docs/delivery/builds-and-releases.md#release-workflow));
the steps below are a developer's build.

First, on the developer's machine, build the image's target with the
[native cross tools](../docs/delivery/builds-and-releases.md), assemble a
server release of the targets in use, and build `xtask` for the builder. For
an arm64 image (use `x86_64-unknown-linux-musl` for amd64), with the Mac's
target for its own runner:

```sh
cargo xtask native build --target aarch64-unknown-linux-musl --target aarch64-apple-darwin
cargo xtask server-release --output .cache/release-<build> \
  --files .cache/release-<build>-files \
  --target aarch64-unknown-linux-musl --target aarch64-apple-darwin
cargo zigbuild --release --locked -p xtask \
  --target aarch64-unknown-linux-musl --target-dir .cache/native-target
```

Then, from the repository root on the builder, with curl, jq, GNU tar, and
util-linux installed:

```sh
sudo bash cloud-guest-image/rootfs/build.sh \
  --xtask .cache/native-target/aarch64-unknown-linux-musl/release/xtask \
  --release .cache/release-<build> --files .cache/release-<build>-files \
  --output <root on the builder>/image
```

`--release` names the server release whose runner and command packages the
image embeds, and `--files` the directory of its files, from which the build
takes the image target's programs and checks each against the release's
manifests. `--output` names a new directory on a Linux filesystem: a release is
immutable, so every build publishes a new one. When the server release lies
on a Linux filesystem of the builder, the output is its own `image/`; a root
on a shared Mac directory cannot hold it, so the builder's manager gets a root
of its own ([Develop on a Mac with Lima](../docs/guides/mac-development.md)).
The script builds the tree in
`/var/tmp/demi-cloud-root`, or in the directory `--work` names, and removes it
after a successful build. `--mirror` names an Ubuntu mirror other than the
official one, such as `https://archive.ubuntu.com/ubuntu` on a builder whose
only way out is HTTPS. apt inside the tree runs with a cleared environment, so
the build hands it two things of the builder's network. A builder that
reaches the mirror only through an HTTPS proxy sets `https_proxy`, which apt
inside the tree then uses as well. A builder whose proxy re-signs TLS names
the bundle that holds the proxy's authority with `--ca FILE`, which apt inside
the tree then trusts, and so does curl when it downloads the container image;
the build removes the file from the tree. The last line the build prints is
the release's base version.
The build neither starts nor resets a Cloud device; see
[Acceptance and local refresh](../docs/cloud/images.md#acceptance-and-local-refresh).

`rootfs/ubuntu.json` pins Ubuntu's container image: the serial and, for each
architecture, the root archive's URL on
`https://partner-images.canonical.com/oci/resolute/`, its size, and its
SHA-256. The build checks the download against it, then brings the packages
up to date from the mirror, so a serial's age does not hold back security
updates. To change the pin, choose a serial listed there, download each
architecture's `ubuntu-resolute-oci-<arch>-root.tar.gz`, check its SHA-256
against the serial's `SHA256SUMS`, and record the serial, URL, size, and
SHA-256.

`rootfs/packages.txt` lists the packages the build installs on the container
image: only what Demi needs in every Cloud and what nearly every task needs.
The agent or the user installs everything else on demand.
