# Set up managed Cloud hosts

Cloud runs on a Linux execution host. The backend reaches the manager over a
restricted Unix socket, and the sandbox's runner reaches the backend over the
backend's runner socket, which the manager mounts into the sandbox. For
development on a Mac, the optional
[Develop on a Mac with Lima](../guides/mac-development.md) guide runs the
manager in a Lima VM.

How the manager works is in the documentation of `crates/machine-manager`
(`src/lib.rs`), and what an image holds is in [Cloud images](images.md).

## Linux requirements

Use a native amd64 or arm64 Linux execution host. The host needs Linux 5.14 or
later, seccomp, namespaces, veth interfaces, OverlayFS, ext4, loop devices,
filesystem freeze, and nftables. With the manager's resource limits on, the
default, it also needs cgroup v2 with the CPU, memory, and PID controllers; a
host without them runs the manager with `DEMI_MANAGED_LIMITS=off`, and its
Clouds then have no CPU, memory, or PID limits. The manager runs as root. An
ordinary hardware-virtualized VPS can provide these facilities without exposing
KVM. A restricted container sold as a VPS may not; check the facilities instead
of relying on the provider's product name.

The manager runs the `runsc` version that `crates/machine-manager/runtime/release.json`
pins, from `/opt/demi/gvisor/<version>/`, where `demi-server` fetches it before
a release that pins it runs. Besides `runsc`, it runs `mke2fs`, `e2fsck`, and
`resize2fs` from e2fsprogs, `bsdtar` from libarchive-tools, and `nft` from
nftables. Docker and containerd are not required. The manager runs as a
privileged system service in a private mount namespace. Only its dedicated
backend/forwarding group may connect to the manager socket; group membership
grants control of Cloud machines.

Before it serves requests, the manager checks the host and refuses to start,
with a named diagnostic, when a requirement is missing; it never falls back to
another runtime or launches an unisolated process.

Existing host firewalls must also permit the manager's approved forwarded
traffic. For example, Docker can leave a `FORWARD` policy of `DROP`; a rule
accepting a packet in the manager's nftables table does not override a later
table's drop. The manager does not rewrite another service's firewall.
Configure that service's integration rules for the Cloud interfaces, then verify
public egress and private destination refusal through a real sandbox.

## Configuration

The manager reads the deployment's one configuration file,
`/opt/demi/config/demi.env`, which the backend reads too
(`crates/backend/src/config/mod.rs`). It reads
four of the backend's settings, and the backend's definition of each holds:

- `DEMI_RELEASE`, the server release root: the manager imports the Cloud image
  in its `image/`. A server leaves it out, so the manager takes the release of
  its own executable, as the backend does.
- `DEMI_BACKEND_PUBLIC_URL`: the URL every boot record must name, the origin
  a Cloud's runner names to its backend.
- `DEMI_MACHINE_MANAGER_SOCKET`: the socket the manager listens on.
- `DEMI_BACKEND_RUNNER_SOCKET`: the backend's runner socket, whose directory
  the manager mounts into each Cloud.

Its own settings carry the prefix `DEMI_MANAGED_`, and it refuses a
`DEMI_MANAGED_*` variable it does not read. Unknown or malformed settings
fail configuration. Counts and MiB sizes are positive decimal integers.

| Variable | Meaning |
| --- | --- |
| `DEMI_MANAGED_DATA` | The manager's persistent state, on one filesystem; default `/opt/demi/data/cloud`. |
| `DEMI_MANAGED_RUNSC` | The pinned runsc executable; default `/opt/demi/gvisor/<pinned version>/runsc`. A deployment leaves it out; the manager's tests name a stand-in. |
| `DEMI_MANAGED_DNS` | The IPv4 resolvers a Cloud uses. Optional: the host's own upstream resolvers otherwise (below). |
| `DEMI_MANAGED_LIMITS` | `on` (default) or `off`: whether sandboxes run under the cgroup v2 CPU, memory, and PID limits (`crates/machine-manager/src/config.rs`). |
| `DEMI_MANAGED_CPUS`, `DEMI_MANAGED_MEM_MIB` | Per-sandbox CPU budget and total memory limit, with the limits on; either one with `DEMI_MANAGED_LIMITS=off` fails configuration. |
| `DEMI_MANAGED_SUBNET`, `DEMI_MANAGED_SLOTS` | Non-overlapping IPv4 address pool and maximum network slots. |

The sizing defaults and limits live in `crates/backend-cloud/src/tuning.rs` and
`crates/machine-manager/src/config.rs`, rather than being duplicated here. The network pool defaults to `172.30.0.0/16` with 256
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
profile turns off (`crates/machine-manager/src/network/mod.rs`). If none
remains, startup fails and names `DEMI_MANAGED_DNS`; the manager never falls
back to a public resolver of its choosing. A host whose resolver is a local
cache with no upstream file, for example, sets the variable.

A reverse proxy in front of the backend, such as the one that serves its
public URL over TLS, must pass each request's `Origin` and `Host` headers to
the backend unchanged. The backend refuses a request from another site by its
`Origin`, and lets a request without one through, so a proxy that drops the
header turns that check off; the backend's log then warns once that the proxy
drops `Origin`. An operator checks a deployment from outside with a request
that names another origin, which must answer 403 `forbidden_origin`:

```sh
curl -s -X POST https://demi.example/api/auth/login \
  -H 'Origin: https://elsewhere.example' \
  -H 'Content-Type: application/json' --data '{}'
```

Any other answer means that the proxy did not pass `Origin`: the backend then
took the login as one from no page, and refused only its empty body, with 400
`invalid_body`. The same request with the product's own origin, such as
`https://demi.example`, must answer 400 `invalid_body`, not 403. Behind a proxy
that rewrites `Host`, the backend refuses the product's own pages, unless
`DEMI_BACKEND_PUBLIC_URL` is their origin (`crates/backend-http/src/gate.rs`
holds the rule).

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
filesystem (`crates/machine-manager/src/storage/mod.rs`); startup
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

The manager's systemd unit is part of the server release, in its `systemd/`,
and `demi-server` copies it into place whenever the server moves to a release
([One release on a server](../delivery/upgrades.md#one-release-on-a-server)). It
runs `/opt/demi/current/bin/demi-machine-manager` as root with the `demi-cloud`
group, private mounts and a restrictive umask, and loads the deployment's
configuration file, `/opt/demi/config/demi.env`. Neither starting nor stopping
the service has a timeout, since a first import, a large recovery or a drain can
take minutes, and the stop runs the manager's recovery afterwards; the unit's
own comments say why (`crates/machine-manager/systemd/demi-machine-manager.service`).
No host shell command supplied by a user becomes a privileged launcher
argument. The manager operates only on its own service, network
namespace/interface names, cgroup subtree, and nftables table.

The backend runs as the system user `demi`, a member of the `demi-cloud`
group, which is what lets it use the manager's socket. A server moves to a
new image, manager and runsc only by moving to a new release
([Upgrades](../delivery/upgrades.md)); a Cloud's system moves to the new
image when its user resets it.

## Acceptance before use

Check that the proxy in front of the backend passes `Origin`, with the request
under [Configuration](#configuration). Start a real
managed device through the backend, not the paired-device claim endpoint. Verify
runner readiness, shell, native and conversation browser operations,
persistent package and home files across stop/wake, and reset with a broken
system. Check network refusal and failure cleanup as the Cloud suite does
(`crates/backend/tests/backend/real_cloud.rs`). Measure each execution host's
performance separately. Results on one host do not certify a different host,
image, runtime, or concurrent-user capacity.
