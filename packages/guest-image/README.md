# Guest image pipeline

The two release artifacts a backend with managed hosts needs
(`docs/demi-next/managed-hosts.md` § Images), built on any Linux with root
(the Lima `fc` instance in development):

```
vmlinux       the guest kernel: Linux 6.1, Firecracker's microvm config plus ours (kernel/)
rootfs.ext4   the shared read-only root: Ubuntu 24.04, the toolchain, the guest user, the runner (rootfs/)
```

```
runner/build.sh   <arch>          → out/<arch>/demi-runner   the packed tinyjs runner for Linux musl (from macOS or Linux)
kernel/build.sh   <arch>          → out/<arch>/vmlinux       (needs the kernel build deps; about 20 minutes on 4 cores)
sudo rootfs/build.sh <arch>       → out/<arch>/rootfs.ext4   (needs debootstrap and network; consumes out/<arch>/demi-runner)
```

`<arch>` is `aarch64` or `x86_64`. Nothing here runs at backend runtime;
the backend is told where the artifacts are (`DEMI_MANAGED_KERNEL`,
`DEMI_MANAGED_ROOTFS`).
