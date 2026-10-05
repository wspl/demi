# Set up managed Cloud hosts

Cloud runs on a Linux execution host. The backend reaches the manager over a
restricted Unix socket and the sandbox's runner connects to the configured
backend endpoint. For development on a Mac, the optional
[Develop on a Mac with Lima](../guides/mac-development.md) guide runs the
manager in a Lima VM.

The authoritative runtime contract is [Managed hosts](managed-hosts.md); image
artifacts are defined in [Cloud images](images.md).

## Linux requirements

Use a native amd64 or arm64 Linux execution host with the runtime release
pinned in [isolation and joining](managed-hosts.md#isolation-and-joining). The
host needs Linux 5.14 or later, seccomp, namespaces, veth interfaces,
OverlayFS, ext4, loop devices, filesystem freeze, and nftables. With the
manager's [resource limits](managed-hosts.md#resource-limits) on, the default,
it also needs cgroup v2 with the CPU, memory, and PID controllers; a host
without them runs the manager with `DEMI_MANAGED_LIMITS=off`, and its Clouds
then have no CPU, memory, or PID limits. The manager runs as root with
`CAP_SYS_RESOURCE`, which the kernel requires to grow a mounted ext4
filesystem. Some container platforms drop that capability even for root; on
such a host a Cloud's disks cannot grow, and each
[growth](managed-hosts.md#lifecycle-and-capacity) fails with an error that
names the capability. An ordinary hardware-virtualized VPS can provide these
facilities without exposing KVM. A restricted container sold as a VPS may not;
check the facilities instead of relying on the provider's product name.

The manager runs the `runsc` version that the runtime release manifest,
`crates/machine-manager/runtime/release.json`, pins, from
`/opt/demi/gvisor/<version>/`, where `demi-server` fetches it before a
release that pins it runs ([gVisor runtime](../delivery/builds-and-releases.md#gvisor-runtime)).
On amd64 the archive is upstream's release, checked against the SHA-512 the
manifest pins. On arm64 it is built from the same pinned source with the
shipped seccomp ABI fix and the native/systrap regression probe, once per
pin, and published as a release of its own whose SHA-512 the manifest pins
too; build dependencies and inputs are in
`crates/machine-manager/runtime/README.md`. Startup requires the executable
to report exactly the pinned version.

The manager is a Linux executable, released for amd64 and arm64
([Builds and releases](../delivery/builds-and-releases.md)). Besides `runsc`, it
runs `mke2fs`, `e2fsck`, and `resize2fs` from e2fsprogs, `bsdtar` from
libarchive-tools, and `nft` from nftables;
[Linux control](managed-hosts.md#linux-control) explains why the manager runs
each one as a program.
Docker and containerd are not required. The manager runs as a privileged system
service in a private mount namespace. Only its dedicated backend/forwarding
group may connect to the manager socket; group membership grants control of
Cloud machines.

Before it serves requests, startup validates root privileges and a private
mount namespace, the programs and the exact runsc version, cgroup enforcement
when the resource limits are on, a state directory on one filesystem, image
architecture and integrity, storage
mount, freeze, and copy support with a probe image, and network policy
installation. A missing requirement fails startup with a named diagnostic. The
manager never falls back to another runtime or launches an unisolated process.

Existing host firewalls must also permit the manager's approved forwarded
traffic. For example, Docker can leave a `FORWARD` policy of `DROP`; a rule
accepting a packet in the manager's nftables table does not override a later
table's drop. The manager does not rewrite another service's firewall.
Configure that service's integration rules for the Cloud interfaces, then verify
public egress and private destination refusal through a real sandbox.

## Configuration

The manager reads the deployment's one configuration file,
`/opt/demi/config/demi.env`, which the backend reads too
([Backend configuration](../backend/backend.md#configuration)). It reads
three of the backend's settings, and the backend's definition of each holds:

- `DEMI_RELEASE`, the [server release](../delivery/builds-and-releases.md#server-release)
  root: the manager imports the Cloud image in its `image/`. A server leaves
  it out, so the manager takes the release of its own executable, as the
  backend does.
- `DEMI_BACKEND_PUBLIC_URL`: the one endpoint, address and port, that a Cloud
  may reach on the host or a private address, and the URL every boot record
  must name.
- `DEMI_MACHINE_MANAGER_SOCKET`: the socket the manager listens on.

Its own settings carry the prefix `DEMI_MANAGED_`, and it refuses a
`DEMI_MANAGED_*` variable it does not read. Unknown or malformed settings
fail configuration. Counts and MiB sizes are positive decimal integers.

| Variable | Meaning |
| --- | --- |
| `DEMI_MANAGED_DATA` | The manager's persistent state, on one filesystem; default `/opt/demi/data/cloud`. |
| `DEMI_MANAGED_RUNSC` | The pinned runsc executable; default `/opt/demi/gvisor/<pinned version>/runsc`. A deployment leaves it out; the manager's tests name a stand-in. |
| `DEMI_MANAGED_DNS` | The IPv4 resolvers a Cloud uses. Optional: the host's own upstream resolvers otherwise (below). |
| `DEMI_MANAGED_LIMITS` | `on` (default) or `off`: whether sandboxes run under the cgroup v2 CPU, memory, and PID limits ([Resource limits](managed-hosts.md#resource-limits)). |
| `DEMI_MANAGED_CPUS`, `DEMI_MANAGED_MEM_MIB` | Per-sandbox CPU budget and total memory limit, with the limits on; either one with `DEMI_MANAGED_LIMITS=off` fails configuration. |
| `DEMI_MANAGED_SYSTEM_MIB`, `DEMI_MANAGED_HOME_MIB` | Initial writable filesystem capacities. |
| `DEMI_MANAGED_SUBNET`, `DEMI_MANAGED_SLOTS` | Non-overlapping IPv4 address pool and maximum network slots. |

The sizing defaults and limits live in
[lifecycle and capacity](managed-hosts.md#lifecycle-and-capacity), rather than
being duplicated here. The network pool defaults to `172.30.0.0/16` with 256
slots, validated against host routes before use. Each slot takes four
addresses, a /30. The pool is written as its network address with a prefix
length from 8 to 30, and it must hold four addresses for every slot; the slot
count ranges from 1 to 16384. Runtime security flags, mount sizes, capability
policy, and process limits are a single shipped profile, not environment
overrides.

`DEMI_MANAGED_DNS` is a comma-separated nonempty list with no loopback,
unspecified, multicast, or broadcast address. Without it, the manager takes
the resolvers the host itself forwards to: those of systemd-resolved's
`/run/systemd/resolve/resolv.conf` when the host runs it, else those of
`/etc/resolv.conf`. It skips the addresses a sandbox cannot use, such as
systemd-resolved's own `127.0.0.53`, and IPv6 addresses, which the sandbox
profile turns off ([Networking](managed-hosts.md#networking)). If none
remains, startup fails and names `DEMI_MANAGED_DNS`; the manager never falls
back to a public resolver of its choosing. A host whose resolver is a local
cache with no upstream file, for example, sets the variable.

A reverse proxy in front of the backend, such as the one that serves its
public URL over TLS, must pass each request's `Origin` and `Host` headers to
the backend unchanged. The backend refuses a request from another site by its
`Origin`, and lets a request without one through, so a proxy that drops the
header turns that check off; the backend's log then warns once that the proxy
drops `Origin`.
[Authentication and ownership](../backend/backend.md#authentication-and-ownership)
gives the rule and the request that checks a deployment.

For example, a server whose backend sits behind Caddy has a configuration
file such as this one; the manager uses its defaults for everything but the
settings it shares with the backend:

```dotenv
DEMI_BACKEND_PUBLIC_URL=https://demi.example.com
DEMI_INSTANCE_MODE=isolated
DEMI_BACKEND_LISTEN=127.0.0.1:3271
```

Public endpoints use TLS. A local HTTP development endpoint is allowed only on
the explicit private path protected by host rules or the development tunnel.

## Storage and service setup

Place persistent state on a dedicated Linux filesystem, and keep the whole state
directory on it. The manager publishes generations by hard-linking images
between its working and image directories, which works only within one
filesystem ([Save a generation](managed-hosts.md#save-a-generation)); startup
refuses a state directory that spans filesystems. XFS with `reflink=1` is the
preferred setup, because a wake and a checkpoint then clone images instead of
copying them; set its copy-on-write extent hint to the filesystem block size for
the state directory. btrfs can clone image files too. On ext4, a wake and a
checkpoint copy the used ranges of both images, with higher latency and space
cost for populated volumes. Hibernation and reset copy no image data on any
filesystem. Never put working images on a shared or network directory or a
container engine's temporary layer.

The installer checks the existing mount and reports an unsuitable configuration;
it never formats a device containing data. Provision a new data volume
separately and point the manager at it. Capacity planning includes working
images, retained generations, and image imports, not just the user-visible
quotas.

A server is set up with `demi-server setup`, which checks these
requirements with the release's own manager and refuses a machine that
fails them ([Installation](../delivery/installation.md)).

The manager's systemd unit is part of the server release, in its
`systemd/`, and `demi-server` copies it into place whenever the server moves
to a release ([One release on a server](../delivery/upgrades.md#one-release-on-a-server)).
It runs `/opt/demi/current/bin/demi-machine-manager` as root with the
`demi-cloud` group, private mounts, and a restrictive umask, and loads the
deployment's configuration file, `/opt/demi/config/demi.env`. The unit uses
`Type=notify`: the manager reports readiness only after it has recovered and
saved leftover devices, installed its network policy, imported its base,
checked storage, and opened its owner/group-restricted socket, so the
installer, an upgrade and every restart see real readiness or a real failure.
Startup has no timeout (`TimeoutStartSec=infinity`), because a first import
or a large recovery can take minutes. `KillMode=mixed` sends the stop signal
to the manager alone, so it drains its devices with its own child processes;
systemd kills whatever remains only after the manager exits. The unit's
stop-post command then runs the manager's recovery (`--recover`), which has
work to do only when the drain did not finish
([Startup and recovery](managed-hosts.md#startup-and-recovery)). Stopping has
no timeout either (`TimeoutStopSec=infinity`): the drain and the recovery
check and publish filesystems, and killing a check midway does more harm than
waiting. No host shell command supplied by a user becomes a privileged
launcher argument. The manager operates only on its own service, network
namespace/interface names, cgroup subtree, and nftables table.

The backend runs as the system user `demi`, a member of the `demi-cloud`
group, which is what lets it use the manager's socket. A server moves to a
new image, manager and runsc only by moving to a new release
([Upgrades](../delivery/upgrades.md)); a Cloud's system moves to the new
image when its user resets it.

## Acceptance before use

Check that the proxy in front of the backend passes `Origin`: a request that
names another origin answers 403 `forbidden_origin` ([Authentication and
ownership](../backend/backend.md#authentication-and-ownership)). Start a real
managed device through the backend, not the paired-device claim endpoint. Verify
runner readiness, shell, native and conversation browser operations,
persistent package and home files across stop/wake, and reset with a broken
system. Check network refusal and failure cleanup using the full [acceptance
contract](managed-hosts.md#verification). Measure each execution host's
performance separately. Results on one host do not certify a different host,
image, runtime, or concurrent-user capacity.
