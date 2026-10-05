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

`demi-server` fetches the pinned archive of the server's architecture,
checks its SHA-512 and unpacks what the manager runs of it into
`/opt/demi/gvisor/<version>/` (`docs/delivery/builds-and-releases.md`
§ gVisor runtime): on amd64 the upstream release, on arm64 the build with
the fix that `.github/workflows/runtime.yml` publishes as the release
`runsc-<arm64Version>`. That workflow runs `../scripts/build-runsc-arm64.sh
<new-directory>` on an arm64 runner; the build needs jq, Git, curl, Python,
build-essential, the amd64 cross compiler, clang, pkg-config, libffi-dev,
libssl-dev, libnuma-dev, and libbpf-dev. On Ubuntu, install
`crossbuild-essential-amd64` for the x86 helpers included in the complete
runtime distribution. Bazel's version and download checksum are pinned; its
source dependencies come from the pinned gVisor tree. No Docker build is
used. The build records its source inputs and archive checksum beside the
executable. The build script reads this directory's `release.json` with jq;
the manager and `demi-server` embed the same file when they are built.

The patch should be removed when a pinned upstream version passes this probe and
the Cloud acceptance suite. Replacing the runtime still requires conversation
browser, persistence, network, resource-limit, and failure-recovery checks on
both Linux architectures.
