# Set up managed Cloud hosts

Cloud runs on a Linux execution host, or inside one Linux VM for local macOS
development. The backend reaches the manager over a restricted Unix socket and
the sandbox's runner connects to the configured backend endpoint.

The authoritative runtime contract is [Managed hosts](demi-next/managed-hosts.md);
image artifacts are defined in [Cloud images](cloud-images.md).

## Linux requirements

Use a native amd64 or arm64 Linux execution host with the runtime release
pinned in [isolation and joining](demi-next/managed-hosts.md#isolation-and-joining).
The host needs seccomp, namespaces, cgroup v2 with CPU/memory/PID controllers, OverlayFS, ext4, loop devices, filesystem freeze, and network
administration through iproute2 and nftables. An ordinary hardware-virtualized
VPS can provide these without exposing KVM. A restricted container sold as a VPS
may not; check the facilities instead of relying on the provider's product name.

The installer installs the complete pinned runsc distribution and verifies its
release checksum on amd64. On arm64 it builds the pinned source with the shipped
seccomp ABI fix and runs the native/systrap regression probe. Build dependencies
and pinned inputs are in [runtime inputs](../packages/machines/runtime/README.md).
Keep `runsc` and its accompanying `gvisor-bin/` directory together; upstream
packaging can include helper binaries. The build/deployment manifest pins the
release and archive hash, and startup validates the configured executable and
required flags. Follow the upstream
[installation instructions](https://gvisor.dev/docs/user_guide/install/).

The execution host also needs Bun for the manager, e2fsprogs, util-linux,
iproute2, nftables, and `bsdtar` from libarchive-tools for the shipped image format. Docker
and containerd are not required. The manager runs as a privileged system service
in a private mount namespace. Only its dedicated backend/forwarding group may
connect to the manager socket; group membership grants control of Cloud machines.

Startup validates runtime support, image architecture and integrity, storage
mount/freeze support, cgroup enforcement, and network policy installation. It
fails with a named diagnostic before serving requests if a requirement is absent.
It never falls back to another runtime or launches an unisolated process.

Existing host firewalls must also permit the manager's approved forwarded traffic.
For example, Docker can leave a `FORWARD` policy of `DROP`; a rule accepting a
packet in the manager's nftables table does not override a later table's drop.
The manager does not rewrite another service's firewall. Configure that service's
integration rules for the Cloud interfaces, then verify public egress and private
destination refusal through a real sandbox.

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
DEMI_MANAGED_RUNSC=/opt/gvisor/<pinned-version>/runsc
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
ext4 uses sparse-preserving copies and has higher save latency and space cost
for populated volumes. Never put
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

After installing the Linux dependencies and publishing an image, install the
service with absolute paths:

```sh
sudo bash packages/machines/scripts/install-managed-hosts.sh \
  --user backend --bun /opt/bun/bin/bun \
  --manager /opt/demi/packages/machines/dist/main.mjs \
  --image /opt/demi-cloud/releases/build-id \
  --backend-url https://backend.example.com --dns 1.1.1.1 \
  --data /var/lib/demi-machines
```

The backend user joins the `demi-cloud` group. Restart its service or login session
to acquire that membership. The installer never starts the backend itself.

Publish a new image, restart the manager, and explicitly reset a device when it
should use the new base. Restart alone does not upgrade pinned devices. A new
runtime starts only after prior writers have stopped. The replacement deployment
uses a new data directory; it does not import or delete older deployment state.

## Mac backend with local Lima

Use one native-architecture Linux VM. On Apple silicon, select arm64 Linux and
arm64 image/native artifacts. The VM runs the same privileged manager and runsc
profile as Linux deployment. It has no nested virtualization requirement.

`packages/machines/lima/demi-machines.yaml` and
`packages/machines/scripts/lima-machines.sh` provision Linux dependencies, a separate persistent
data disk, manager service, network policy, and Unix socket forwarding. The Mac
backend connects at `~/.lima/demi-machines/sock/demi-machines.sock`; the script
prints the guest-reachable Mac URL to configure. Verify the connection by
starting a managed device through the backend.
Do not assume a particular Lima gateway address works on every installation.

With an image already built inside Lima:

```sh
bash packages/machines/scripts/lima-machines.sh \
  --image /opt/demi-cloud/releases/build-id --dns 1.1.1.1
```

For an existing VM, pass `--data` with a newly prepared Linux state directory.
The installer does not reinterpret an old deployment's disk layout.

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
Lima and VPS performance separately. The [evaluation report](gvisor-evaluation.md)
records tested profiles; those observations do not certify a different host,
image, runtime, or concurrent-user capacity.
