# Cloud image build

A Cloud image release is an architecture-specific Linux root archive and its
manifest for gVisor. [Cloud images](../docs/cloud/images.md) defines the
format, the build pipeline, import, and acceptance;
[Cloud setup](../docs/cloud/setup.md) describes deployment.

A build runs as root on a Linux builder of the image's architecture.
`rootfs/build.sh` makes the Ubuntu tree, then runs `release cloud-image package`,
which embeds the runner release, the command packages, Chrome for Testing, and
uv, and publishes the release.

First, on the developer's machine, build and package the image's target with
the [Go cross builds](../docs/delivery/builds-and-releases.md), and build
`tools/release` for the builder. For an arm64 image (use
`x86_64-unknown-linux-musl` and `GOARCH=amd64` for an amd64 builder):

```sh
export CGO_ENABLED=0

go run ./tools/release native build --target aarch64-unknown-linux-musl
go run ./tools/release native package --package demi-runner \
  --target aarch64-unknown-linux-musl --output .cache/releases/runners
go run ./tools/release native package --package demi-file \
  --target aarch64-unknown-linux-musl --output .cache/releases/demi-file-<build>
go run ./tools/release native package --package demi-browser \
  --target aarch64-unknown-linux-musl --output .cache/releases/demi-browser-<build>
go run ./tools/release native package --package demi-claude-code \
  --target aarch64-unknown-linux-musl --output .cache/releases/demi-claude-<build>
GOOS=linux GOARCH=arm64 go build -trimpath -ldflags='-s -w' \
  -o .cache/release-linux-arm64 ./tools/release
```

Then, from the repository root on the builder, with debootstrap, GNU tar, and
util-linux installed:

```sh
sudo bash cloud-guest-image/rootfs/build.sh \
  --release-tool .cache/release-linux-arm64 \
  --runners .cache/releases/runners \
  --package .cache/releases/demi-file-<build> \
  --package .cache/releases/demi-browser-<build> \
  --package .cache/releases/demi-claude-<build> \
  --output /opt/demi-cloud/releases/<build>
```

`--output` names a new directory on a Linux filesystem: a release is
immutable, so every build publishes a new one. The script builds the tree in
`/var/tmp/demi-cloud-root`, or in the directory `--work` names, and removes it
after a successful build. `--mirror` names an Ubuntu mirror other than the
official one, such as `https://archive.ubuntu.com/ubuntu` on a builder whose
only way out is HTTPS. apt inside the tree runs with a cleared environment, so
the build hands it two things of the builder's network. A builder that
reaches the mirror through a proxy sets `http_proxy` or `https_proxy`, which
apt inside the tree then uses as well; the tree's `sources.list` still names
the mirror itself. A builder whose proxy re-signs TLS names
the bundle that holds the proxy's authority with `--ca FILE`, which apt inside
the tree then trusts; the build removes the file from the tree. The last line the build prints is the release's base version.

Most of a build's time is apt downloading the toolchain, about 380 MB for
arm64. A builder that builds often runs a local package cache and points
`http_proxy` at it; a second build then downloads nothing from the mirror:

```sh
sudo apt-get install -y apt-cacher-ng
sudo http_proxy=http://127.0.0.1:3142 bash cloud-guest-image/rootfs/build.sh ...
```

The build neither starts nor resets a Cloud device; see
[Acceptance and local refresh](../docs/cloud/images.md#acceptance-and-local-refresh).

`rootfs/uv.json` pins uv: its version and, for each architecture, the
archive's URL, size, SHA-256, and executables. To change the pin, download each
archive, check its SHA-256 against the uv release's published digest, record
it, run `CGO_ENABLED=0 go generate ./tools/release`
from the repository root to refresh the generated embedded copy, and rebuild
the release tool, which carries the pin it was built with.
