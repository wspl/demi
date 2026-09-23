# Set up managed Cloud hosts

Cloud runs on a Linux execution host, or inside one Linux VM for local macOS
development. The backend reaches the manager over a restricted Unix socket and
the sandbox's runner connects to the configured backend endpoint.

The authoritative runtime contract is [Managed hosts](managed-hosts.md); image
artifacts are defined in [Cloud images](images.md).

## Linux requirements

Use a native amd64 or arm64 Linux execution host with the runtime release
pinned in [isolation and joining](managed-hosts.md#isolation-and-joining). The
host needs Linux 5.14 or later, seccomp, namespaces, veth interfaces, cgroup v2
with CPU/memory/PID controllers, OverlayFS, ext4, loop devices, filesystem
freeze, and nftables. An ordinary hardware-virtualized VPS can provide these
without exposing KVM. A restricted container sold as a VPS may not; check the
facilities instead of relying on the provider's product name.

The installer installs the complete pinned runsc distribution and verifies its
release checksum on amd64. On arm64 it builds the pinned source with the shipped
seccomp ABI fix and runs the native/systrap regression probe. Build dependencies
and pinned inputs are in `crates/machines/runtime/README.md`. Keep `runsc` and
its accompanying `gvisor-bin/` directory together; upstream packaging can
include helper binaries. The runtime release manifest pins the release and
archive hash, and startup requires the configured executable to report exactly
the pinned version. Follow the upstream
[installation instructions](https://gvisor.dev/docs/user_guide/install/).

The manager is a Linux executable, released for amd64 and arm64
([Builds and releases](../delivery/builds-and-releases.md)). Besides `runsc`, it
runs `mke2fs`, `e2fsck`, and `resize2fs` from e2fsprogs, `bsdtar` from
libarchive-tools, and `nft` from nftables;
[Linux control](managed-hosts.md#linux-control) explains why the manager runs
each one as a program. The install scripts also need `jq` to read the pinned
runtime release.
Docker and containerd are not required. The manager runs as a privileged system
service in a private mount namespace. Only its dedicated backend/forwarding
group may connect to the manager socket; group membership grants control of
Cloud machines.

Before it serves requests, startup validates root privileges and a private
mount namespace, the programs and the exact runsc version, cgroup enforcement,
a state directory on one filesystem, image architecture and integrity, storage
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

The manager and the backend each validate their configuration at startup.
Unknown or malformed managed settings fail configuration: the manager refuses
any `DEMI_MANAGED_*` variable it does not know. Counts and MiB sizes are
positive decimal integers. Cloud is mandatory, so a missing manager
configuration is a startup error.

| Variable | Owner and meaning |
| --- | --- |
| `DEMI_MACHINES_SOCKET` | Manager listen path; backend connect path. Different paths are expected when forwarded. |
| `DEMI_MACHINES_DATA` | Manager's persistent state, on one filesystem; default `/var/lib/demi-machines`. |
| `DEMI_MANAGED_RUNSC` | Required absolute path to the pinned runsc executable. |
| `DEMI_MANAGED_IMAGE` | Required directory containing the Cloud image manifest and archive. |
| `DEMI_MANAGED_CPUS`, `DEMI_MANAGED_MEM_MIB` | Per-sandbox CPU budget and total memory limit. |
| `DEMI_MANAGED_SYSTEM_MIB`, `DEMI_MANAGED_HOME_MIB` | Initial writable filesystem capacities. |
| `DEMI_MANAGED_SUBNET`, `DEMI_MANAGED_SLOTS` | Non-overlapping IPv4 address pool and maximum network slots. |
| `DEMI_MANAGED_DNS` | Required reachable IPv4 resolver addresses, validated as an address list. |
| `DEMI_MANAGED_BACKEND_URL` | Manager's allowed runner destination; each wake must match it. |
| `DEMI_BACKEND_PUBLIC_URL` | Backend's runner-reachable URL; must name the same endpoint as the manager allowlist. |

The sizing defaults and limits live in
[lifecycle and capacity](managed-hosts.md#lifecycle-and-capacity), rather than
being duplicated here. The network pool defaults to `172.30.0.0/16` with 256
slots, validated against host routes before use. Each slot takes four
addresses, a /30. The pool is written as its network address with a prefix
length from 8 to 30, and it must hold four addresses for every slot; the slot
count ranges from 1 to 16384. `DEMI_MANAGED_DNS` is a comma-separated nonempty
list with no loopback, unspecified, multicast, or broadcast address.
`DEMI_MANAGED_BACKEND_URL` uses `http` or `https`. Runtime security flags,
mount sizes, capability policy, and process limits are a single shipped
profile, not environment overrides.

The backend's own settings, among them the manager socket, the public URL, and
its native release configuration, are defined in
[Backend configuration](../backend/backend.md#configuration). An image release
and the backend's command releases must agree.

Example manager configuration for an already prepared Linux execution host:

```dotenv
DEMI_MACHINES_SOCKET=/run/demi-cloud/machines.sock
DEMI_MACHINES_DATA=/var/lib/demi-machines
DEMI_MANAGED_RUNSC=/opt/gvisor/<pinned-version>/runsc
DEMI_MANAGED_IMAGE=/opt/demi-cloud/current
DEMI_MANAGED_BACKEND_URL=https://backend.example.com
DEMI_MANAGED_DNS=1.1.1.1,8.8.8.8
```

Resolver addresses above are examples, not a product requirement. Choose
permitted resolvers reachable from the actual sandbox. Public endpoints use TLS.
A local HTTP development endpoint is allowed only on the explicit private path
protected by host rules or the development tunnel.

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
filesystem. Never put working images on a Mac shared directory or a container
engine's temporary layer.

The installer checks the existing mount and reports an unsuitable configuration;
it never formats a device containing data. Provision a new data volume
separately and point the manager at it. Capacity planning includes working
images, retained generations, and image imports, not just the user-visible
quotas.

The installer writes a systemd unit that runs the manager binary as root with
the `demi-cloud` group, private mounts, and a restrictive umask. The unit uses
`Type=notify`: the manager reports readiness only after it has recovered and
saved leftover devices, installed its network policy, imported its base,
checked storage, and opened its owner/group-restricted socket, so the installer
and every restart see real readiness or a real failure. Startup has no timeout
(`TimeoutStartSec=infinity`), because a first import or a large recovery can
take minutes. `KillMode=mixed` sends the stop signal to the manager alone, so it
drains its devices with its own child processes; systemd kills whatever remains
only after the manager exits. The unit's stop-post command then runs the
manager's recovery (`--recover`), which has work to do only when the drain did
not finish ([Startup and recovery](managed-hosts.md#startup-and-recovery)). No
host shell command supplied by a user becomes a privileged launcher argument.
The installer operates only on its own service, network namespace/interface
names, cgroup subtree, and nftables table.

After installing the Linux dependencies and the manager binary and publishing an
image, install the service with absolute paths:

```sh
sudo bash crates/machines/scripts/install-managed-hosts.sh \
  --user backend \
  --manager /opt/demi/bin/demi-machines \
  --image /opt/demi-cloud/releases/build-id \
  --backend-url https://backend.example.com --dns 1.1.1.1 \
  --data /var/lib/demi-machines
```

`--manager` names the manager executable. The backend user joins the
`demi-cloud` group. Restart its service or login session to acquire that
membership. The installer never starts the backend itself.

Publish a new image, restart the manager, and explicitly reset a device when it
should use the new base. Restart alone does not upgrade pinned devices. A new
runtime starts only after prior writers have stopped.

## Mac backend with local Lima

Use one native-architecture Linux VM. On Apple silicon, select arm64 Linux and
arm64 image/native artifacts. The VM runs the same privileged manager and runsc
profile as Linux deployment. It has no nested virtualization requirement.

`crates/machines/lima/demi-machines.yaml` and
`crates/machines/scripts/lima-machines.sh` provision Linux dependencies, a
separate persistent data disk, manager service, network policy, and Unix socket
forwarding. The service runs the manager built for the VM's architecture, which
the Mac cross-compiles with its own tools
([Builds and releases](../delivery/builds-and-releases.md)). The Mac backend
connects at `~/.lima/demi-machines/sock/demi-machines.sock`; the script prints
the guest-reachable Mac URL to configure. Verify the connection by starting a
managed device through the backend. Do not assume a particular Lima gateway
address works on every installation.

With an image already built inside Lima:

```sh
bash crates/machines/scripts/lima-machines.sh \
  --image /opt/demi-cloud/releases/build-id --dns 1.1.1.1
```

The script prepares its default state directory on the Lima data disk. To use
another one, pass `--data` with a prepared Linux state directory.

Data stays on the Linux disk. Stopping or recreating the Lima instance preserves
that disk; deleting the data disk is a separate explicit action. Lima formats
the data disk only while it is blank, and the scripts never reformat storage or
reuse another deployment's state directory to make a start succeed.

## Acceptance before use

Start a real managed device through the backend, not the paired-device claim
endpoint. Verify runner readiness, shell/native/browser operations, persistent
package and home files across stop/wake, and reset with a broken system. Check
network refusal and failure cleanup using the full
[acceptance contract](managed-hosts.md#verification). Measure local Lima and VPS
performance separately. Results on one host do not certify a different host,
image, runtime, or concurrent-user capacity.
