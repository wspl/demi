# Set up managed Cloud hosts

Cloud runs on a Linux execution host:

- The backend reaches the machine manager over a restricted Unix socket.
- The runner in each sandbox reaches the backend over the backend's runner
  socket, which the manager mounts into the sandbox.

Related reading:

- How the manager works: `crates/machine-manager/src/lib.rs`.
- What an image holds: [Cloud images](images.md).
- Development on a Mac, with the manager in a Lima VM:
  [Develop on a Mac with Lima](../guides/mac-development.md).

## Linux requirements

### The host

- A native amd64 or arm64 Linux host.
- Linux 5.14 or later.
- seccomp, namespaces, veth interfaces, OverlayFS, ext4, loop devices,
  filesystem freeze and nftables.
- cgroup v2 with the CPU, memory and PID controllers, while the manager's
  resource limits are on (the default). A host without them runs the manager
  with `DEMI_MANAGED_LIMITS=off`, and its Clouds then have no CPU, memory or
  PID limits.

An ordinary hardware-virtualized VPS provides these without exposing KVM. A
restricted container sold as a VPS may not: check the facilities, not the
provider's product name.

### Programs

- `runsc`, the version `crates/machine-manager/runtime/release.json` pins,
  from `/opt/demi/gvisor/<version>/`. `demi-server` fetches it before a
  release that pins it runs.
- `mke2fs`, `e2fsck` and `resize2fs` from e2fsprogs.
- `bsdtar` from libarchive-tools.
- `nft` from nftables.

Docker and containerd are not required.

### How the manager runs

- As root, as a privileged system service in a private mount namespace.
- Only its dedicated group may connect to its socket. Membership in that
  group grants control of Cloud machines.
- Before it serves requests, it checks the host. When a requirement is
  missing, it refuses to start with a named diagnostic. It never falls back
  to another runtime or starts an unisolated process.

### Other firewalls

Existing host firewalls must permit the manager's forwarded traffic:

- Docker, for example, can leave a `FORWARD` policy of `DROP`. A rule that
  accepts a packet in the manager's nftables table does not override a later
  table's drop.
- The manager does not rewrite another service's firewall. Configure that
  service's rules for the Cloud interfaces.
- Then check, through a real sandbox, that public egress works and private
  destinations are refused.

## Configuration

The manager reads the deployment's one configuration file,
`/opt/demi/config/demi.env`, which the backend reads too
(`crates/backend/src/config/mod.rs`).

### Settings shared with the backend

The backend's definition of each holds.

| Variable | What the manager does with it |
| --- | --- |
| `DEMI_RELEASE` | Imports the Cloud image in the release's `image/`. A server leaves it out: the manager takes its own executable's release, as the backend does. |
| `DEMI_BACKEND_PUBLIC_URL` | Requires every boot record to name it: the origin a Cloud's runner names to its backend. |
| `DEMI_MACHINE_MANAGER_SOCKET` | Listens on it. |
| `DEMI_BACKEND_RUNNER_SOCKET` | Mounts its directory into each Cloud. |

### The manager's own settings

- They carry the prefix `DEMI_MANAGED_`.
- An unknown `DEMI_MANAGED_*` variable, or a malformed value, fails
  configuration.
- Counts and MiB sizes are positive decimal integers.

| Variable | Meaning |
| --- | --- |
| `DEMI_MANAGED_DATA` | The manager's persistent state, on one filesystem; default `/opt/demi/data/cloud`. |
| `DEMI_MANAGED_RUNSC` | The pinned runsc executable; default `/opt/demi/gvisor/<pinned version>/runsc`. A deployment leaves it out; the manager's tests name a stand-in. |
| `DEMI_MANAGED_DNS` | The IPv4 resolvers a Cloud uses. Optional: see [DNS](#dns). |
| `DEMI_MANAGED_LIMITS` | `on` (default) or `off`: whether sandboxes run under the cgroup v2 CPU, memory and PID limits. |
| `DEMI_MANAGED_CPUS`, `DEMI_MANAGED_MEM_MIB` | Per-sandbox CPU budget and total memory limit, with the limits on. Either one with `DEMI_MANAGED_LIMITS=off` fails configuration. |
| `DEMI_MANAGED_SUBNET`, `DEMI_MANAGED_SLOTS` | The IPv4 address pool and the maximum number of network slots. |

The sizing defaults and limits are in `crates/backend-cloud/src/tuning.rs`
and `crates/machine-manager/src/config.rs`.

### The network pool

- Default: `172.30.0.0/16` with 256 slots, checked against the host's routes
  before use.
- Each slot takes four addresses, a /30.
- The pool is written as its network address with a prefix length from 8
  to 30, and must hold four addresses for every slot.
- The slot count ranges from 1 to 16384.

Runtime security flags, mount sizes, capabilities and process limits are one
shipped profile, not settings.

### DNS

`DEMI_MANAGED_DNS` is a comma-separated, nonempty list. It holds no loopback,
unspecified, multicast or broadcast address.

Without it, the manager takes the resolvers the host forwards to:

1. Those in systemd-resolved's `/run/systemd/resolve/resolv.conf`, when the
   host runs it.
2. Otherwise those in `/etc/resolv.conf`.

From those, it skips the addresses a sandbox cannot use
(`crates/machine-manager/src/network/mod.rs`):

- the addresses the variable may not hold either: loopback (such as
  systemd-resolved's own `127.0.0.53`), unspecified, multicast and
  broadcast;
- IPv6 addresses, which the sandbox turns off.

If none remains, startup fails and names `DEMI_MANAGED_DNS`. The manager
never picks a public resolver itself. So a host whose resolver is a local
cache with no upstream file sets the variable.

### A reverse proxy

A reverse proxy in front of the backend, such as the one that serves its
public URL over TLS, must pass each request's `Origin` and `Host` headers
unchanged:

- The backend refuses a request from another site by its `Origin`, and lets
  a request without one through. A proxy that drops the header turns that
  check off; the backend's log then warns once.
- Behind a proxy that rewrites `Host`, the backend refuses the product's own
  pages, unless `DEMI_BACKEND_PUBLIC_URL` is their origin
  (`crates/backend-http/src/gate.rs`).

To check a deployment from outside, send a request that names another
origin:

```sh
curl -s -X POST https://demi.example/api/auth/login \
  -H 'Origin: https://elsewhere.example' \
  -H 'Content-Type: application/json' --data '{}'
```

- It must answer 403 `forbidden_origin`.
- Any other answer means that the proxy dropped `Origin`: the backend took
  the login as one from no page and refused only its empty body, with 400
  `invalid_body`.
- The same request with the product's own origin, such as
  `https://demi.example`, must answer 400 `invalid_body`, not 403.

### Example

A server whose backend sits behind Caddy has a configuration file such as
this one. The manager uses its defaults for everything but the settings it
shares with the backend.

```dotenv
DEMI_BACKEND_PUBLIC_URL=https://demi.example.com
DEMI_INSTANCE_MODE=isolated
DEMI_BACKEND_LISTEN=127.0.0.1:3271
```

Public endpoints use TLS. A local HTTP development endpoint is allowed only
on a private path that host rules or the development tunnel protect.

## Storage and service setup

### The state filesystem

Keep the whole state directory on one dedicated Linux filesystem:

- The manager publishes generations by hard-linking images between its
  working and image directories, which works only within one filesystem
  (`crates/machine-manager/src/storage/mod.rs`).
- Startup refuses a state directory that spans filesystems.

| Filesystem | A wake and a checkpoint |
| --- | --- |
| XFS with `reflink=1` (preferred) | Clone images. Set the state directory's copy-on-write extent hint to the filesystem block size. |
| btrfs | Clone images. |
| ext4 | Copy the used ranges of both images: slower, and more space for populated volumes. |

- Hibernation and reset copy no image data on any filesystem.
- Never put working images on a shared or network directory, or on a
  container engine's temporary layer.
- The installer checks the existing mount and reports an unsuitable one. It
  never formats a device that holds data: provision a new data volume
  separately and point the manager at it.
- Plan capacity for working images, retained generations and image imports,
  not only the quotas users see.

`demi-server setup` checks these requirements with the release's own manager
and refuses a machine that fails them
([Installation](../delivery/installation.md)).

### The systemd unit

The manager's unit is part of the server release, in its `systemd/`.
`demi-server` copies it into place whenever the server moves to a release
([One release on a server](../delivery/upgrades.md#one-release-on-a-server)).

The unit:

- runs `/opt/demi/current/bin/demi-machine-manager` as root, with the
  `demi-cloud` group, private mounts and a restrictive umask;
- loads `/opt/demi/config/demi.env`;
- has no start or stop timeout: a first import, a large recovery or a drain
  can take minutes, and the stop runs the manager's recovery afterwards
  (`crates/machine-manager/systemd/demi-machine-manager.service` says why).

The manager touches only its own service, network namespace and interface
names, cgroup subtree and nftables table. No shell command a user supplies
becomes an argument of a privileged program.

### The backend and upgrades

- The backend runs as the system user `demi`, a member of the `demi-cloud`
  group, which lets it use the manager's socket.
- A server gets a new image, manager and runsc only by moving to a new
  release ([Upgrades](../delivery/upgrades.md)).
- A Cloud's system moves to the new image when its user resets it.

## Acceptance before use

1. Check that the proxy in front of the backend passes `Origin`, with the
   request under [A reverse proxy](#a-reverse-proxy).
2. Start a real managed device through the backend, not through the
   paired-device claim endpoint.
3. Check:
   - runner readiness;
   - shell, native and conversation browser operations;
   - packages and home files that persist across stop and wake;
   - reset of a broken system.
4. Check network refusal and failure cleanup as the Cloud suite does
   (`crates/backend/tests/backend/real_cloud.rs`).
5. Measure each execution host's performance separately. Results on one host
   do not certify another host, image, runtime or concurrent-user capacity.
