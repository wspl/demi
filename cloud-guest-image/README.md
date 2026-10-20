# Cloud image build

A Cloud image release is an architecture-specific Linux root archive and its
manifest for gVisor. [Cloud images](../docs/cloud/images.md) defines the
format, the build pipeline, import, and acceptance.
[Cloud setup](../docs/cloud/setup.md) describes deployment.

A build runs as root on a Linux builder of the image's architecture.
`rootfs/build.sh` does two things:

1. Makes the Ubuntu tree from Ubuntu's container image.
2. Runs `xtask cloud-image package`, which embeds the runner release and the
   command packages' executables, and publishes the release.

The image goes into the `image/` directory of the server release
(`crates/xtask/src/server_release.rs`) whose runner and command package
releases it embeds.

The release workflow builds the published images this way on its `ubuntu-26.04`
and `ubuntu-26.04-arm` runners (`.github/workflows/release.yml`). The rest of
this page describes a developer's build.

## Build on the developer's machine

First, do these steps on the developer's machine:

1. Build the image's target with the native cross tools
   (`crates/xtask/src/native/build.rs`).
2. Assemble a server release of the targets in use.
3. Build `xtask` for the builder.

The following commands build an arm64 image, with the Mac's target for its own
runner. Use `x86_64-unknown-linux-musl` for amd64.

```sh
bun xtask native build --target aarch64-unknown-linux-musl --target aarch64-apple-darwin
bun xtask server-release --output .cache/release-<build> \
  --files .cache/release-<build>-files \
  --target aarch64-unknown-linux-musl --target aarch64-apple-darwin
cargo zigbuild --release --locked -p xtask \
  --target aarch64-unknown-linux-musl --target-dir .cache/native-target
```

## Build on the builder

Then run the following command from the repository root on the builder, with
curl, jq, GNU tar, and util-linux installed:

```sh
sudo bash cloud-guest-image/rootfs/build.sh \
  --xtask .cache/native-target/aarch64-unknown-linux-musl/release/xtask \
  --release .cache/release-<build> --files .cache/release-<build>-files \
  --output <root on the builder>/image
```

### Build script options

The script takes these options:

- `--release` names the server release whose runner and command packages the
  image embeds.
- `--files` names the directory of that release's files. The build takes the
  image target's programs from it and checks each against the release's
  manifests.
- `--output` names a new directory on a Linux filesystem. A release is
  immutable, so every build publishes a new one.
- `--work` names the directory where the script builds the tree. The default is
  `/var/tmp/demi-cloud-root`. The script removes the tree after a successful
  build.
- `--mirror` names an Ubuntu mirror other than the official one, such as
  `https://archive.ubuntu.com/ubuntu` on a builder whose only way out is HTTPS.
- `--ca FILE` names the bundle that holds the authority of a proxy that
  re-signs TLS. apt inside the tree trusts it, and so does curl when it
  downloads the container image. The build removes the file from the tree.

When the server release lies on a Linux filesystem of the builder, the output
is its own `image/`. A root on a shared Mac directory cannot hold it, so the
builder's manager gets a root of its own
([Develop on a Mac with Lima](../docs/guides/mac-development.md)).

apt inside the tree runs with a cleared environment, so the build hands it two
things of the builder's network:

- A builder that reaches the mirror only through an HTTPS proxy sets
  `https_proxy`, which apt inside the tree then uses as well.
- A builder whose proxy re-signs TLS passes `--ca FILE`, which the `--ca`
  option describes.

The last line the build prints is the release's base version.

The build neither starts nor resets a Cloud device. For that, see
[Acceptance and local refresh](../docs/cloud/images.md#acceptance-and-local-refresh).

## The Ubuntu pin

`rootfs/ubuntu.json` pins these parts of Ubuntu's container image:

- The serial
- For each architecture, the root archive's URL on
  `https://partner-images.canonical.com/oci/resolute/`, its size and its
  SHA-256

The build checks the download against the pin, then brings the packages up to
date from the mirror, so a serial's age does not hold back security updates.

To change the pin:

1. Choose a serial listed at that URL.
2. Download each architecture's `ubuntu-resolute-oci-<arch>-root.tar.gz`.
3. Check its SHA-256 against the serial's `SHA256SUMS`.
4. Record the serial, URL, size, and SHA-256.

## The package list

`rootfs/packages.txt` lists the packages the build installs on the container
image: only what Demi needs in every Cloud and what nearly every task needs.
The agent or the user installs everything else on demand.
