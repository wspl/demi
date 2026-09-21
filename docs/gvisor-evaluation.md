# gVisor systrap evaluation

Evaluations performed on 2026-09-22. The direct manager results below follow the
initial OCI feasibility experiment recorded later in this document. The runtime
contract is [Managed hosts](demi-next/managed-hosts.md).

## Direct manager acceptance

The replacement manager was exercised on native Linux arm64 inside Lima and
native Linux amd64 on the VPS, with no Docker runtime and no KVM requirement.
Both used the shipped two-CPU, 2 GiB sandbox profile, real managed runner
credentials, the release root archive, and the backend's normal Host access.
The arm64 execution VM had four CPUs and 8 GiB RAM; the amd64 VPS had two CPUs
and 4 GiB RAM. State storage was XFS with reflinks on Lima and ext4 on the VPS.
Automated model responses were scripted; runner, native commands, Chrome, storage,
and networking were real.

| Observation | Local arm64 | VPS amd64 |
| --- | ---: | ---: |
| First command through the managed runner | 0.32 s | 3.21 s |
| Chrome open | 1.20 s | 5.82 s |
| Viewer connection to first live frame | 0.21 s | 0.76 s |
| Paired filesystem checkpoint | 0.076 s | 0.388 s |

These are separate individual observations with the base already imported, not
cold-image download timings or latency percentiles. The arm64 browser measurement
came from the running development backend opening a blank page; the VPS suite
opened its local HTML fixture and included the remote test connection. The arm64
checkpoint used two 1 GiB filesystems; the VPS checkpoint followed growth to
2 GiB each. No concurrent-user capacity or memory-peak benchmark is implied.

Both architectures passed managed shell/native execution, UID/file ownership,
sudo, browser frames and input, public HTTPS and private/metadata refusal,
online growth, checkpoint, stop/wake persistence, reset after disabling bash,
and rustup/nvm installation followed by later cargo/node commands. The updated
Mac paired device also opened Chrome and delivered live frames through the
running backend. The local development Cloud was reset onto the new base.

Fault injection on arm64 froze a working filesystem and killed its manager;
recovery preserved the newest system/home pair. A separate systemd test killed
the manager with a frozen storage probe, then read a previously written Cloud
file through the still-running backend. The saved mount namespace enabled thaw
and cleanup; the backend queried runtime state and woke the recovered Cloud
without requiring a backend restart. A separate amd64 checkpoint test read the
immutable saved image while a live process still held dirty, unflushed mmap
pages; the saved file contained the new bytes.

### Defects found during implementation

- Upstream ARM gVisor overwrote X0 in a `SECCOMP_RET_TRAP` signal frame. Chrome's
  scheduler syscall handler expects the original first argument and crashed when
  live capture started. A native-Linux versus systrap C probe demonstrated the
  difference. The pinned ARM build restores the original argument; the probe and
  live browser pass without disabling Chrome's sandbox. Pinned source, patch,
  hashes, and build instructions live in
  [runtime inputs](../packages/machines/runtime/README.md).
- Bun's file-copy fallback allocated the empty ranges of the volume files on
  ext4. A first command took 51.5 s in that run. Sparse-preserving GNU copies
  reduced the later observed first-command time to 3.2 s on the same VPS;
  reflink-capable filesystems still clone. A Linux regression test checks both
  allocated blocks and data at the boundaries of a sparse image.
- The service's restrictive umask also affected the sandbox root and resolver
  files. New root directories and read-only configuration now receive explicit
  sandbox-visible modes. Existing user root-directory modes remain unchanged.
- The VPS's Docker firewall dropped forwarded traffic from the new interfaces.
  Test-scoped integration rules allowed the manager-approved packets through
  that existing firewall. Cloud's own private-destination rules remained active.
  Deployment requirements are documented in [Cloud setup](managed-hosts-setup.md).

The full managed lifecycle and login-tool acceptance is reproducible through
`real-gvisor.e2e.test.ts`; Linux storage tests cover sparse copies and restrictive
umasks. Host power-loss behavior and sustained concurrent-user capacity remain
separate acceptance work. The runtime does not implement memory snapshots or
multi-worker failover.

## Initial OCI feasibility experiment

### Result

gVisor's systrap platform can run Demi's existing Linux runner, native browser
driver and live browser capture on a VPS without KVM. The tested application
binaries required no changes. Chrome's own sandbox remained enabled. Shell,
sudo, Node.js, Python virtual environments, a small C compilation, Git writes,
PTY creation, outbound HTTPS, npm registry access, browser inspection,
screenshots, live input, and Chinese/Japanese/Korean rendering worked after the
configuration changes below.

These results informed the selected gVisor-backed Cloud design. They do not
establish production readiness, a multi-user isolation guarantee, or equivalent performance
to Firecracker on a physical Linux host.

### Environment and method

- Execution host: an x86_64 VPS, two vCPUs, about 4 GiB RAM, Debian 13,
  kernel `6.12.95+deb13-cloud-amd64`; no `/dev/kvm` or exposed VMX.
- Docker `29.6.1`; gVisor `release-20260914.0`, explicitly `--platform=systrap`.
  The release archive was checked against its published SHA-512 digest.
- Containers: two CPUs, 2 GiB memory limit, 256 MiB shared memory, an init
  process, an ordinary `demi` user, and a private bridge network.
- Evaluation image: Ubuntu 26.04, the repository's
  [guest package list](../packages/guest-image/rootfs/packages.txt), Chrome for
  Testing `153.0.8010.36`, and freshly cross-compiled x86_64 musl binaries.
  This is an OCI evaluation image, not a boot of the Firecracker ext4 image;
  the guest kernel/PID 1 initialization and standalone `uv` were not included.
- Source revision: `272990f3bf6ed085d13ef1ddbb1fbc29587b9104`.
- The diagnostic backend used a temporary database and scripted provider.
  Temporary connections carried the runner and fixture website. The ordinary
  development backend and existing Cloud were not switched in this experiment.
- The existing runner used the paired-device registration path for this
  experiment. Browser commands and user streams used normal conversation Host
  access. This tested real runner/command-service/browser transports, not a new
  managed-device boot or a mock browser. No real model was called.

The same OCI image was used for the runc control. Default Docker runc security
settings refused Chrome's sandbox with `No usable sandbox`. Only the runc
control then used `seccomp=unconfined` and `apparmor=unconfined`; Chrome's sandbox
was still enabled. The gVisor runs used neither setting, no privileged mode,
no additional `SYS_ADMIN` capability, and no host-network mode. This is a native
execution baseline, not a comparison between equivalent isolation boundaries.

### Observations

The complete corrected gVisor run passed all diagnostic commands, live input,
frame delivery and font checks. The fixture animated at 10 Hz, at 800 by 600
pixels and device pixel ratio 1. Frame counts therefore do not measure maximum
encoding throughput.

| Operation | gVisor, corrected configuration | runc control |
| --- | ---: | ---: |
| First native `browser tabs` | 0.64 s | 0.38 s |
| First `browser open about:blank` | 5.11 s | 2.65 s |
| Open again after closing the last tab | 2.59 s | 0.61 s |
| Inspect the blank page | 0.24–0.29 s | 0.04 s |
| Screenshot | 0.11–0.19 s | 0.05–0.06 s |
| Close the last tab and retire Chrome | 0.53–0.57 s | 0.18–0.20 s |
| Live view connection to first encoded frame | 1.51 s | 0.79 s |
| Frames received over the next five seconds | 51 | 51 |
| Container memory sampled during live view | 343 MiB | 199 MiB |
| Create a Python virtual environment | 3.96 s | 2.80 s |
| Compile and run a trivial C program | 0.12 s | 0.04 s |
| Initialize a Git repository and commit one file | 0.11 s | 0.01 s |

These are individual observations, not percentiles or a capacity benchmark.
Both table samples include the explicit resolver and font checks. Earlier
correctly initialized gVisor trials observed first opens of 5.13 and 7.56 s,
reopens of 3.13 and 2.69 s, and first live frames around 0.96–0.97 s.
Host page caches were not cleared: “first” means a new container/native service,
not a cold physical disk. Command timings were measured inside the container;
live-view timings included the Mac, SSH tunnel and network. Memory is one Docker
cgroup sample, not peak usage or a per-user sizing guarantee.

The local ARM Firecracker failure investigated earlier ran on different
hardware, architecture and nested virtualization. These results show that this
VPS can support a usable browser through gVisor; they do not establish a numerical
speedup over Firecracker or identify its low-level slowdown.

### Required configuration discovered by the tests

#### Preserve system writes

The default `--overlay2=root:self` failed the Cloud persistence requirement.
After writing a marker under `/etc`, stopping the container and starting the same
container, the marker was gone. A separate home volume retained its marker.
With `--overlay2=none`, both markers survived.

The same stop/restart check passed on the full evaluation image. Removing that
container and creating a clean one from the image with the same home volume
removed the system marker and retained the home marker. Runtime start took
0.25 s and restart 0.26 s, measured locally on the VPS. Those measurements start
an idle container, not the runner registration or first browser operation.

The runtime must write into a persistent per-user writable root, with a separate
home volume. Reset must select a clean system root while retaining home. Disabling
gVisor's overlay does not by itself implement atomic generations, snapshots,
quota growth, crash recovery or the backend's reset journal. See the
[gVisor filesystem documentation](https://gvisor.dev/docs/user_guide/filesystem/).

#### Permit sudo within the sandbox

Without `--allow-suid=true`, `sudo -n id` failed because effective UID did not
become zero. Enabling it made the command return UID 0 inside the sandbox. This
matches the current Cloud's passwordless-sudo requirement. It is not host root
access and does not grant every Linux capability. The deployed runtime's
`runsc flags` documents this option and its interaction with OCI
`no-new-privileges`.

#### Reap orphaned processes

Using `sleep` as container PID 1 left orphaned Chrome/crashpad zombies. Demi's
process-tree cleanup then failed after about five seconds and retained the
profile. Docker's `--init` reaped them: close succeeded in under one second and
the subsequent process listing contained no Chrome or crashpad processes.

A production container needs an explicit supervisor/init contract. At the time of this initial experiment, the
runner treated Linux PID 1 as Firecracker guest initialization; using it unchanged
as container PID 1 would enter the wrong boot path.

#### Supply reachable DNS

The Docker custom bridge's embedded loopback resolver was unreachable through
gVisor's isolated network stack. `curl https://example.com` and `npm view` failed
name resolution even though the fixture website worked by IP.

An explicit resolver file with external DNS addresses, mounted read-only at
`/etc/resolv.conf`, restored both operations without host networking. The
[gVisor FAQ](https://gvisor.dev/docs/user_guide/faq/#my-container-cannot-resolve-another-containers-name-when-using-docker-user-defined-bridge)
describes the embedded-resolver limitation. Production needs its own reachable
DNS and egress policy, including the existing private/link-local restrictions.

### Decision boundary

The initial evaluation justified implementing the selected
[replacement design](demi-next/managed-hosts.md) on the existing Host contract.
It did not validate managed-token delivery, atomic disk generations, growth,
abrupt manager failure, or the direct OCI launch path. Subsequent managed checks
are recorded above. No fallback isolation mode follows from these measurements.

Nested Docker, workloads requiring additional kernel capabilities, sandbox escape
testing, and an independent security audit were outside this experiment. Its
network configuration was diagnostic connectivity, not the deployed manager's
egress policy.

### Evidence and reproduction inputs

Local scripts and raw results are retained under `.cache/gvisor-eval/`:
`evaluation.test.ts`, `guest.py`, `lifecycle.py`, `systrap.log`,
`systrap-init.log`, `systrap-final.log`, `systrap-dns.log`, `runc.log`,
`runc-control.log`, `runc-final.log`, and the lifecycle logs. These ignored diagnostic files are
not part of the distributed product or a portable test suite.

The remote evaluation directory is `/home/admin/demi-gvisor-eval`; its runtime,
image and build logs are retained for follow-up. Runtime configuration used:

```text
--platform=systrap --overlay2=none --allow-suid=true
```

All evaluation containers, their temporary home volumes, SSH tunnels and relays
were removed or stopped after the tests. The installed runtime registrations and
the OCI image remain; Docker's default runtime was not changed.

Artifacts used by both runtime controls:

| Artifact | SHA-256 |
| --- | --- |
| OCI image | `676560364a21601839ec4e8269f8a564d542c75fc739827d763016b66c5b8310` |
| Runner | `cda52162fa4d69656696d3ef0107ec0c2109fd0d05429e84944afd7539cbc5e8` |
| Native commands | `c1032084f2dd4e212de6372eb157668879ee88171eeebd6bb103568477196368` |
| Chrome executable | `79a4ebf6da53e4ceab11844257aabc5166f17b595dc694d6382cbee8ff50565f` |

The official [platform guide](https://gvisor.dev/docs/architecture_guide/platforms/)
recommends systrap for VMs and hosts without virtualization support. That makes it
an appropriate candidate; the application results above provide the project-specific
evidence.
