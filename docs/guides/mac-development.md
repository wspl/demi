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
Build and package those two targets with the Mac's own cross tools and its
Apple SDK ([Builds and releases](../delivery/builds-and-releases.md)):

```sh
cargo xtask native build \
  --sdk /Library/Developer/CommandLineTools/SDKs/MacOSX<version>.sdk \
  --target aarch64-apple-darwin --target aarch64-unknown-linux-musl
cargo xtask native package --package demi-runner --output .cache/releases/runners \
  --target aarch64-apple-darwin --target aarch64-unknown-linux-musl
cargo xtask native package --package demi-file --output .cache/releases/demi-file \
  --target aarch64-apple-darwin --target aarch64-unknown-linux-musl
cargo xtask native package --package demi-browser --output .cache/releases/demi-browser \
  --target aarch64-apple-darwin --target aarch64-unknown-linux-musl
cargo xtask native package --package demi-claude-code --output .cache/releases/demi-claude-code \
  --target aarch64-apple-darwin --target aarch64-unknown-linux-musl
```

## Cloud image in Lima

Build the image inside the VM as the
[guest image build](../../cloud-guest-image/README.md) describes, prefixing
the build command with `limactl shell demi-machine-manager --`. The VM sees the Mac's
home directory at the same path. Keep the image's output and the manager's
working images on the VM's Linux disk, never on the shared Mac directory.

## Machine manager in Lima

`crates/machine-manager/lima/demi-machine-manager.yaml` and
`crates/machine-manager/scripts/lima-machines.sh` provision the Linux dependencies, a
separate persistent data disk, the manager service, the network policy, and
Unix socket forwarding. The VM runs the same privileged manager and runsc
profile as a Linux execution host. The script copies the manager built for the
VM's architecture into the VM under its SHA-256, so a later build never
replaces the executable of a running manager.

```sh
cargo xtask native build --package demi-machine-manager --target aarch64-unknown-linux-musl
cargo xtask native package --package demi-machine-manager \
  --target aarch64-unknown-linux-musl --output .cache/releases/demi-machine-manager-<build>
bash crates/machine-manager/scripts/lima-machines.sh \
  --manager .cache/releases/demi-machine-manager-<build>/aarch64-unknown-linux-musl/demi-machine-manager \
  --image /opt/demi-cloud/releases/build-id \
  --backend-url http://<the Mac's address>:3271 --dns 1.1.1.1
```

`--backend-url` is the backend's public URL. The Cloud guests connect to it
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
script again with the new URL and restart the backend with it.

`--root <directory>` passes on to the installer, which then only writes the
unit and settings beneath that directory inside the VM. The script prepares
its default state directory on the Lima data disk; `--data` names another
prepared Linux state directory.

Stopping or recreating the Lima instance preserves the data disk; deleting it
is a separate explicit action. Lima formats the data disk only while it is
blank, and the scripts never reformat storage or reuse another deployment's
state directory to make a start succeed.

## Backend on the Mac

Follow the [Development backend](../backend/backend.md#development-backend)
steps with the two targets above, and start the backend with the two
settings the script printed:

```sh
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
