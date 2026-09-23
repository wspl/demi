# Cloud image pipeline

The selected pipeline produces an architecture-specific Linux root archive and
manifest for gVisor/systrap. Build inputs, artifact format, import, and acceptance
are defined in [Cloud images](../../docs/cloud/images.md). Deployment is described
in [Cloud setup](../../docs/cloud/setup.md).

Build the runner and command packages with the native cross tools first. On a
matching Linux builder with Bun, debootstrap, Python, zstd, tar, and util-linux:

```sh
sudo env PATH="$PATH" \
  DEMI_CLOUD_RUNNER=/path/to/demi-runner \
  DEMI_CLOUD_BUILD_DIR=/var/tmp/demi-cloud-build \
  DEMI_CLOUD_IMAGE_OUTPUT=/opt/demi-cloud/releases/new-build \
  ROOTFS_WORK=/var/tmp/demi-cloud-root \
  bash rootfs/build.sh aarch64 \
    --runner-release /path/to/runner-release \
    --package /path/to/demi-builtin-release \
    --package /path/to/demi-claude-release
```

Use `x86_64` on an amd64 builder. Output directories are immutable: use a new
release path for every build. Keep work and output on a Linux filesystem when
the source checkout is shared from a Mac. The script creates the root archive
and manifest together; it neither starts nor resets a Cloud device.
