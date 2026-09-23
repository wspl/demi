# Cloud runtime inputs

`release.json` pins the upstream source, amd64 distribution checksum, ARM patch,
and Bazel bootstrap. The runtime profile belongs to
[Managed hosts](../../../docs/cloud/managed-hosts.md#isolation-and-joining).

The ARM patch corrects the signal frame for `SECCOMP_RET_TRAP`. Linux preserves
X0, which holds the first syscall argument. Upstream gVisor writes the syscall
number there instead. Chrome uses the original argument to emulate a trapped
`sched_getaffinity` call; the incorrect value makes its seccomp handler crash.
No seccomp rule or Chrome sandbox feature is disabled by this fix.

`seccomp-trap.c` reproduces that ABI difference without Chrome. It traps a
`getpid` call with a recognizable first argument and checks the signal frame.
The ARM build runs it on native Linux and under the compiled systrap runtime.

Build on native Linux ARM with `scripts/build-runsc-arm64.sh <new-directory>`.
The host needs Bun, Git, curl, Python, build-essential, the amd64 cross compiler,
clang, pkg-config, libffi-dev, libssl-dev, libnuma-dev, and libbpf-dev. On Ubuntu,
install `crossbuild-essential-amd64` for the x86 helpers included in the complete
runtime distribution. Bazel's version and download checksum are pinned; its
source dependencies come from the pinned gVisor tree. No Docker build is used.
The build records its source inputs and archive checksum beside the executable.

The patch should be removed when a pinned upstream version passes this probe and
the Cloud acceptance suite. Replacing the runtime still requires browser,
persistence, network, resource-limit, and failure-recovery checks on both Linux
architectures.
