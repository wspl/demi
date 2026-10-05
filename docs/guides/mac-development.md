# Develop on a Mac with Lima

This guide is optional. The standard development machine is Linux
([Development backend](../backend/backend.md#development-backend)); nothing in
the product or its checks needs a Mac. Use this guide only to develop on a Mac.

gVisor runs only on Linux, so on a Mac the machine manager and its Clouds run
inside one Lima Linux VM with the Mac's own CPU architecture. systrap needs no
nested virtualization. The backend, its data and the web application stay on
the Mac, and the Mac is also a paired device:

```text
Mac
  backend, web application, runner (paired device)
    | Unix socket forwarded by Lima
    v
Lima VM (Linux, the Mac's architecture)
  machine manager -> gVisor/systrap sandboxes -> Cloud guest runner
    |
    +---- outbound connection to the backend at the Mac's address
```

## Hosts in use

On an Apple silicon Mac, the Hosts in use are the Mac,
`aarch64-apple-darwin`, and the Cloud guest, `aarch64-unknown-linux-musl`.
Build those two targets with the Mac's own toolchain, cross tools and Apple
SDK ([Builds and releases](../delivery/builds-and-releases.md)), and assemble
a server release of them, which the backend on the Mac uses:

```sh
cargo xtask native build \
  --sdk /Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk \
  --target aarch64-apple-darwin --target aarch64-unknown-linux-musl
cargo xtask server-release --output .cache/release-<build> \
  --files .cache/release-<build>-files \
  --target aarch64-apple-darwin --target aarch64-unknown-linux-musl
```

The root has no `bin/` and no `image/`: the backend runs from the Cargo
target directory, and the image belongs to the machine manager's own root in
the VM, on the VM's Linux disk.

## Cloud image in Lima

Build the image inside the VM as the
[guest image build](../../cloud-guest-image/README.md) describes, from the
root above, prefixing the build command with
`limactl shell demi-machine-manager --`. The VM sees the Mac's home directory
at the same path. Its output is the `image/` of the manager's root on the
VM's Linux disk, such as `/opt/demi/dev-<build>/image`, never a shared Mac
directory.

## Machine manager in Lima

`crates/machine-manager/lima/demi-machine-manager.yaml` and
`crates/machine-manager/scripts/lima-machines.sh` provision the Linux dependencies, a
separate persistent data disk, the manager service, the network policy, and
Unix socket forwarding. The VM runs the same privileged manager and runsc
profile as a Linux execution host. `--release` names the manager's root in
the VM, the one whose `image/` the image build wrote. The script copies the
manager and `demi-server` built for the VM's architecture into that root's
`bin/`, renaming each into place so that a running manager keeps its own file, puts the manager's
unit in its `systemd/`, as a server release carries it, has `demi-server
runtime` fetch the pinned gVisor into `/opt/demi/gvisor/`, writes the VM's
configuration file with the public URL, the socket and the state directory,
and installs the service from the root through `/opt/demi/current`.

```sh
cargo xtask native build --package demi-machine-manager --package demi-server \
  --target aarch64-unknown-linux-musl
bash crates/machine-manager/scripts/lima-machines.sh \
  --manager .cache/native-target/aarch64-unknown-linux-musl/release/demi-machine-manager \
  --server .cache/native-target/aarch64-unknown-linux-musl/release/demi-server \
  --release /opt/demi/dev-<build> \
  --public-url http://<the Mac's address>:3271
```

`--public-url` is the backend's public URL. The Cloud guests connect to it
through Lima's network, and the Mac's own runner connects to it directly, so
it names the Mac's address on its network, such as Wi-Fi's
`192.168.75.36`. Lima's gateway address, `host.lima.internal` or
`192.168.5.2`, exists only inside the VM: the Mac cannot reach it. For
Wi-Fi, this prints the address:

```sh
ipconfig getifaddr en0
```

The script prints the backend's two settings: the forwarded socket and the
public URL. When the Mac joins another network, its address changes: run the
script again with the new URL and restart the backend with it. The Clouds use
the VM's own upstream resolver, the manager's default, unless `--dns
<addresses>` names others. A proxy on the Mac that answers name lookups with
its own addresses in `198.18.0.0/15`, as a proxy in fake-IP mode does, needs
it: the manager's network policy refuses that range to the Clouds, so every
name a Cloud looks up would be unreachable. `--dns 1.1.1.1` then gives the
Clouds a resolver that answers with the real addresses.

The [Cloud suite](../delivery/scenarios.md#cloud-suite) runs in the VM, as
root, with programs built for it on the Mac: build the manager, `demi-server`
and the backend's scenario test executable for the VM's Linux target with the
Mac's cross tools, then run `cloud-suite.sh` in the VM with `--programs`
naming the VM's release root, a whole server release (`commands/`,
`runners/` and `release.json` beside `image/`), since the suite's backend
publishes its command packages. The suite's manager and the installed one
cannot run at once, since both own the same firewall table: stop the installed
service for the run and start it again afterwards. The script refuses to start
while a manager runs; the table a stopped manager leaves it replaces, and the
installed manager makes its own again when it starts. The suite's manager has
a state directory and a socket of its own, so the installed manager's Clouds
stay as they were.

`--root <directory>` passes on to the installer, which then only writes the
unit and the link beneath that directory inside the VM. The script prepares its default
state directory on the Lima data disk; `--data` names another prepared Linux
state directory.

Stopping or recreating the Lima instance preserves the data disk; deleting it
is a separate explicit action. Lima formats the data disk only while it is
blank, and the scripts never reformat storage or reuse another deployment's
state directory to make a start succeed.

## Backend on the Mac

Follow the [Development backend](../backend/backend.md#development-backend)
steps with the root above, and start the backend with the two settings the
script printed:

```sh
DEMI_RELEASE=.cache/release-<build> \
DEMI_BACKEND_PUBLIC_URL=http://<the Mac's address>:3271 \
DEMI_MACHINE_MANAGER_SOCKET=~/.lima/demi-machine-manager/sock/demi-machine-manager.sock \
...
```

Pair the Mac with the installer at the same URL,
`curl -fsSL http://<the Mac's address>:3271/install.sh | sh`. Verify the
connection by starting a Cloud through the backend.

## Machine manager tests

The manager builds only for Linux, so on a Mac its tests are cross-built with
cargo-zigbuild and run in the VM. The tests that need root run with
`--include-ignored` under sudo:

```sh
cargo zigbuild --tests --target aarch64-unknown-linux-musl \
  -p demi-machine-manager -p demi-machine-manager-protocol --target-dir .cache/linux-target
limactl shell demi-machine-manager -- <test executable>
limactl shell demi-machine-manager -- sudo <test executable> --include-ignored
```

## Limits of a Lima Cloud

- A Lima Cloud is not an acceptance environment: results on it do not certify
  a Linux execution host, and its performance says nothing about one.
- Lima's `vz` VMs do not pass SME through, so the live view captures on Apple
  M4 and later. A VM that passes SME without SVE through cannot capture
  ([Live view](../browser/live-view.md)).
