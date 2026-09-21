# gVisor systrap evaluation

Evaluation performed on 2026-09-22. This is evidence for a deployment decision,
not an implemented Cloud provisioner. The authoritative Cloud contract remains
[Managed hosts](demi-next/managed-hosts.md).

## Result

gVisor's systrap platform can run Demi's existing Linux runner, native browser
driver and live browser capture on a VPS without KVM. The tested application
binaries required no changes. Chrome's own sandbox remained enabled. Shell,
sudo, Node.js, Python virtual environments, a small C compilation, Git writes,
PTY creation, outbound HTTPS, npm registry access, browser inspection,
screenshots, live input, and Chinese/Japanese/Korean rendering worked after the
configuration changes below.

This supports proceeding to a gVisor-backed Cloud design. It does not establish
production readiness, a multi-user isolation guarantee, or equivalent performance
to Firecracker on a physical Linux host.

## Environment and method

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
- The test backend ran separately on the Mac, on port 3288 with a temporary
  database and scripted provider. SSH reverse forwards and bridge-bound relays
  carried the runner connection and fixture website. The ordinary backend on
  port 3271 and the existing Cloud were not switched.
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

## Observations

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

## Required configuration discovered by the tests

### Preserve system writes

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

### Permit sudo within the sandbox

Without `--allow-suid=true`, `sudo -n id` failed because effective UID did not
become zero. Enabling it made the command return UID 0 inside the sandbox. This
matches the current Cloud's passwordless-sudo requirement. It is not host root
access and does not grant every Linux capability. The deployed runtime's
`runsc flags` documents this option and its interaction with OCI
`no-new-privileges`.

### Reap orphaned processes

Using `sleep` as container PID 1 left orphaned Chrome/crashpad zombies. Demi's
process-tree cleanup then failed after about five seconds and retained the
profile. Docker's `--init` reaped them: close succeeded in under one second and
the subsequent process listing contained no Chrome or crashpad processes.

A production container needs an explicit supervisor/init contract. The current
runner treats Linux PID 1 as Firecracker guest initialization; using it unchanged
as container PID 1 would enter the wrong boot path.

### Supply reachable DNS

The Docker custom bridge's embedded loopback resolver was unreachable through
gVisor's isolated network stack. `curl https://example.com` and `npm view` failed
name resolution even though the fixture website worked by IP.

An explicit resolver file with external DNS addresses, mounted read-only at
`/etc/resolv.conf`, restored both operations without host networking. The
[gVisor FAQ](https://gvisor.dev/docs/user_guide/faq/#my-container-cannot-resolve-another-containers-name-when-using-docker-user-defined-bridge)
describes the embedded-resolver limitation. Production needs its own reachable
DNS and egress policy, including the existing private/link-local restrictions.

## Decision boundary

The next step is a `ManagedHostProvisioner` design and implementation for this
runtime, keeping backend, agent and browser code on the existing Host contract.
The evaluation does not justify silently falling back to another isolation mode.

Before accepting that implementation, verify managed-token delivery, automatic
wake, stop/reset admission, retained home, atomic system/home generations,
disk quota/growth, abrupt failure recovery, process/cgroup cleanup, and concurrent
users. Exercise package installation, substantial repository/build workloads,
and the product's browser panel at representative resolutions. Nested Docker
and workloads requiring additional kernel capabilities were not evaluated.

The network rules used here are diagnostic connectivity, not the finished
multi-user egress policy. No sandbox escape testing or independent security audit
was performed. The existing Firecracker provisioner remains available; replacing
its deployment contract requires a separate design decision.

## Evidence and reproduction inputs

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
