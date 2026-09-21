# Set up managed Cloud hosts

The backend can run on your Mac while a Linux VPS executes Cloud. The Mac sends
lifecycle requests through a forwarded manager socket; the sandbox's runner
connects back through a separate endpoint. The same split applies when Linux
runs locally inside Lima.

This guide describes the selected gVisor deployment. **The installer, manager
adapter, image pipeline, and Lima template still need implementation.** The
configuration below is their intended interface, not a claim that the current
scripts accept it. The authoritative runtime contract is
[Managed hosts](demi-next/managed-hosts.md); image artifacts are defined in
[Cloud images](cloud-images.md).

## Linux requirements

Use a native amd64 or arm64 Linux execution host with the runtime release
pinned in [isolation and joining](demi-next/managed-hosts.md#isolation-and-joining).
The host needs seccomp, namespaces, cgroup v2 with CPU/memory/PID controllers, OverlayFS, ext4, loop devices, filesystem freeze, and network
administration through iproute2 and nftables. An ordinary hardware-virtualized
VPS can provide these without exposing KVM. A restricted container sold as a VPS
may not; check the facilities instead of relying on the provider's product name.

Install the complete pinned runsc distribution and verify its release checksum.
Keep `runsc` and its accompanying `gvisor-bin/` directory together; upstream
packaging can include helper binaries. The build/deployment manifest pins the
release and archive hash, and startup validates the configured executable and
required flags. Follow the upstream
[installation instructions](https://gvisor.dev/docs/user_guide/install/).

The execution host also needs Bun for the manager, e2fsprogs, util-linux,
iproute2, nftables, and an archive reader for the shipped image format. Docker
and containerd are not required. The manager runs as a privileged system service
in a private mount namespace. Only its dedicated backend/forwarding group may
connect to the manager socket; group membership grants control of Cloud machines.

Startup validates runtime support, image architecture and integrity, storage
mount/freeze support, cgroup enforcement, and network policy installation. It
fails with a named diagnostic before serving requests if a requirement is absent.
It never falls back to another runtime or launches an unisolated process.

## Configuration

Both processes validate their environment at startup. Unknown or malformed
managed settings fail configuration; required values are not inferred from a
legacy runtime switch. Counts and MiB sizes are positive integers. Cloud is
mandatory, so a missing manager configuration is a startup error.

| Variable | Owner and meaning |
| --- | --- |
| `DEMI_MACHINES_SOCKET` | Manager listen path; backend connect path. Different paths are expected when forwarded. |
| `DEMI_MACHINES_DATA` | Manager's persistent state, default `/var/lib/demi-machines`. |
| `DEMI_MANAGED_RUNSC` | Required absolute path to the pinned runsc executable. |
| `DEMI_MANAGED_IMAGE` | Required directory containing the Cloud image manifest and archive. |
| `DEMI_MANAGED_CPUS`, `DEMI_MANAGED_MEM_MIB` | Per-sandbox CPU budget and total memory limit. |
| `DEMI_MANAGED_SYSTEM_MIB`, `DEMI_MANAGED_HOME_MIB` | Initial writable filesystem capacities. |
| `DEMI_MANAGED_SUBNET`, `DEMI_MANAGED_SLOTS` | Non-overlapping IPv4 address pool and maximum network slots. |
| `DEMI_MANAGED_DNS` | Required reachable IPv4 resolver addresses, validated as an address list. |
| `DEMI_MANAGED_BACKEND_URL` | Manager's allowed runner destination; each wake must match it. |
| `DEMI_BACKEND_PUBLIC_URL` | Backend's runner-reachable URL; must name the same endpoint as the manager allowlist. |

The sizing defaults and limits live in
[lifecycle and capacity](demi-next/managed-hosts.md#lifecycle-and-capacity), rather
than being duplicated here. The network pool defaults to `172.30.0.0/16` with
256 slots, validated against host routes before use. `DEMI_MANAGED_DNS` is a
comma-separated nonempty list with no loopback or unspecified address. Runtime
security flags, mount sizes, capability policy, and process limits are a single
shipped profile, not environment overrides. All old hypervisor/kernel/jailer
variables are removed; there is no `direct`/`jailer` selector.

The backend independently requires a valid `DEMI_BACKEND_PORT`,
`DEMI_INSTANCE_MODE` (`shared` or `isolated`), manager socket, and public URL.
Its [native release configuration](demi-next/native-runtime.md#backend-deployment-configuration)
remains required. An image release and backend command release must agree.

Example manager configuration for an already prepared Linux execution host:

```dotenv
DEMI_MACHINES_SOCKET=/run/demi/machines.sock
DEMI_MACHINES_DATA=/var/lib/demi-machines
DEMI_MANAGED_RUNSC=/opt/gvisor/runsc
DEMI_MANAGED_IMAGE=/opt/demi-cloud/current
DEMI_MANAGED_BACKEND_URL=https://backend.example.com
DEMI_MANAGED_DNS=1.1.1.1,8.8.8.8
```

Resolver addresses above are examples, not a product requirement. Choose permitted
resolvers reachable from the actual sandbox. Public endpoints use TLS. A local
HTTP development endpoint is allowed only on the explicit private path protected
by host rules or the development tunnel.

## Storage and service setup

Place persistent state on a dedicated Linux filesystem. XFS with `reflink=1` is
the preferred image-cloning setup; set its copy-on-write extent hint to the
filesystem block size for the state directory. btrfs can clone image files too.
ext4 works by copying and has higher save latency and space cost. Never put
working images on a Mac shared directory or a container engine's temporary layer.

The installer checks the existing mount and reports an unsuitable configuration;
it never formats a device containing data. Provision a new data volume separately
and point the manager at it. Capacity planning includes working images, retained
generations, and image imports, not just the user-visible quotas.

The service creates an owner/group-restricted socket, acquires the state lock,
reconciles surviving runtimes and mounts, and installs its network policy before
readiness. Its systemd unit keeps the manager's mounts private and defines process
cleanup on service death. No host shell command supplied by a user becomes a
privileged launcher argument. The installer operates only on its own service,
network namespace/interface names, cgroup subtree, and nftables table.

Publish a new image, restart the manager, and explicitly reset a device when it
should use the new base. Restart alone does not upgrade pinned devices. A new
runtime starts only after prior writers have stopped. The replacement deployment
uses a new data directory; it does not import or delete older deployment state.

## Mac backend with a remote VPS

There are two independent transports:

```text
Mac backend -> local Unix socket -> SSH -> VPS manager socket
VPS sandbox -> VPS private relay -> SSH reverse forward -> Mac backend
```

For example, keep the Mac backend listening on `127.0.0.1:3271`. Forward its
manager connection from a private Mac socket to `/run/demi/machines.sock` on the
VPS using SSH stream-local forwarding. Use a dedicated SSH account that can open
that socket. The SSH forward is the socket's authorization boundary on the Mac;
keep its parent directory private.

Create a reverse TCP forward from VPS loopback `127.0.0.1:4287` to Mac
`127.0.0.1:3271`. A supervised relay on the VPS's sandbox gateway listens on
port 4288 and forwards to that loopback port. Configure the same gateway URL in
both `DEMI_BACKEND_PUBLIC_URL` and `DEMI_MANAGED_BACKEND_URL`. Permit only the
sandbox interfaces to reach the relay. Do not bind it to the VPS public address
or enable unrestricted SSH `GatewayPorts`.

The deployment helper owns SSH, relay, socket, and firewall lifetimes together.
Use exit-on-forward-failure, liveness detection, and bounded reconnects. A broken
tunnel makes Cloud unavailable; do not allocate a replacement device. A broken
control connection is reconciled before retrying a transition. Remove forwarding
sockets and relays on cancellation or helper exit, including failed setup.

This arrangement keeps the database, conversations, agent, and web service on the
Mac, and persistent Cloud files on the VPS. It is a development topology, not
multi-worker failover. No conversation shell/file operation uses SSH; those
continue through the runner and normal Host access.

## Mac backend with local Lima

Use one native-architecture Linux VM. On Apple silicon, select arm64 Linux and
arm64 image/native artifacts. The VM runs the same privileged manager and runsc
profile as Linux deployment. It has no nested virtualization requirement.

The replacement `packages/machines/lima/demi-machines.yaml` and
`scripts/lima-machines.sh` must provision Linux dependencies, a separate persistent
data disk, manager service, network policy, and Unix socket forwarding. The Mac
backend connects at `~/.lima/demi-machines/sock/demi-machines.sock`; the script
prints the guest-reachable Mac URL to configure and validates it from a sandbox.
Do not assume a particular Lima gateway address works on every installation.

If a checkout is shared for manager development, it belongs only to the outer
Linux VM, never to a user's sandbox. Data stays on the Linux disk. Stopping or
recreating the Lima instance preserves that disk; deleting the data disk is a
separate explicit action. The script must not reuse or reformat an old instance's
storage to make this replacement appear to start successfully.

## Acceptance before use

Start a real managed device through the backend, not the paired-device claim
endpoint. Verify runner readiness, shell/native/browser operations, persistent
package and home files across stop/wake, and reset with a broken system. Check
network refusal and failure cleanup using the full
[acceptance contract](demi-next/managed-hosts.md#verification). Measure local
Lima and VPS performance separately; the existing x86 VPS evaluation establishes
neither arm64 performance nor acceptance of this manager implementation.
