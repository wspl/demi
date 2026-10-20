# Installation

One command installs a Demi server on a Linux machine, and a person or an AI
agent can run it. The command puts `demi-server` on the machine and goes on
into its `setup`, which sets the server up. `setup` installs the layout that
[Upgrades](upgrades.md#one-release-on-a-server) defines, and the server moves
between releases with `demi-server upgrade` from then on.

Every choice is a parameter of `setup`. What happens to a missing parameter
depends on who runs the command:

- A person at a terminal is asked for what is missing.
- An agent runs commands without a terminal. It gets the guide to the
  parameters, `setup --help`, and runs `setup` again with them.

For example, an agent installs Demi on a fresh Ubuntu server, behind a
reverse proxy on the same machine that serves `demi.example.com` over HTTPS
and forwards to port 3271:

```sh
curl -fsSL https://demi.sh/install.sh | sudo bash
# no terminal: prints the guide; the agent reads it and runs
sudo demi-server setup --domain demi.example.com --mode isolated --listen 127.0.0.1:3271
```

When a person runs the first command in a terminal, the command asks for the
domain, the mode and the listening address instead, and sets the server up
with the answers.

`https://demi.sh/install.sh` redirects to
`https://github.com/wspl/demi/releases/latest/download/install.sh`, the latest
release's `install.sh`. Either address installs the same release.

A deploy's pre-releases are never the latest
([A development build on a server](upgrades.md#a-development-build-on-a-server)).
Two options install another release:

- `--prerelease` installs the newest release, pre-releases included.
- Any release's own `install.sh` installs that release.

The following commands show both options:

```sh
curl -fsSL https://demi.sh/install.sh | sudo bash -s -- --prerelease
curl -fsSL https://github.com/wspl/demi/releases/download/v0.1.22/install.sh | sudo bash
```

While no release is published, the first address answers 404 and only the
second works.

`setup` takes these steps:

1. Checks the machine.
2. Installs the few system packages the machine manager needs.
3. Fetches the newest release.
4. Writes the configuration.
5. Starts the backend and the machine manager.
6. Checks the server from outside.

It ends by printing `https://demi.example.com`, where the first visitor creates
the master account on the setup page (`packages/web/src/auth/session.ts`).
Until the account exists, anyone who reaches the domain can create it, so the
operator opens the page right away.

Installation is Linux only, on the [supported distributions](#distributions).
`setup` refuses a machine that cannot run Cloud, because every deployment has
Cloud. There is no uninstall.

## The bootstrap

`install.sh` is an asset of every release, beside `demi-server-<target>` for
the two Linux targets (`.github/workflows/release.yml`). It takes one parameter
of its own, `--prerelease`, and does only what a shell must:

1. Checks that it runs as root on Linux.
2. Picks the target from the machine's architecture.
3. Downloads that `demi-server` and the release's `SHA256SUMS` from the release
   it came from, or with `--prerelease` from the newest release, pre-releases
   included, and checks the one against the other.
4. Installs `demi-server` as `/usr/local/bin/demi-server`.
5. Runs `demi-server setup` with the parameters it was given
   (`bash -s -- <parameters>`). Its input is the session's terminal,
   `/dev/tty`, when there is one, because its own input is the script.

On a machine that has a Demi server already, `install.sh` changes nothing and
points to `demi-server upgrade`.

Everything else is `demi-server`'s, the program that upgrades the server later,
so setting up and upgrading share the fetch, the layout and the units. A setup
is the move from nothing to the first release: the release of the
`demi-server` that runs it, unless `--version` names another.

After `setup` has started that release, `/usr/local/bin/demi-server` becomes a
link to `/opt/demi/current/bin/demi-server`, so the server's program on the
path is always the current release's.

## A person or an agent

No program can tell a person from an agent. Whether a terminal is there to
answer comes close, so `setup` decides by it, as the following table shows:

| `setup` runs | With every required parameter | Without |
| --- | --- | --- |
| With a terminal | Sets the server up, asking nothing | Asks for each missing parameter, offering the defaults of the optional ones, then sets the server up |
| Without a terminal, as an agent runs it | Sets the server up | Prints the guide, `--help`, and exits with 0: the guide is the answer, not a failure |

`--no-input` makes a run with a terminal behave as one without. An agent
that already knows the parameters passes them to the first command, as
`bash -s -- --domain …`, and `setup` runs at once.

## Parameters

`demi-server setup --help` lists every parameter with its meaning, which
parameters each HTTPS choice needs, and complete examples. An agent reads it
and composes the command.

The following table lists the parameters:

| Parameter | Meaning |
| --- | --- |
| `--domain <name>` | Required. The domain the product is reached at. The public URL is `https://<name>`. `setup` refuses an IP address (`crates/backend/src/config/mod.rs`). |
| `--mode shared\|isolated` | Required, with no default (`crates/backend-providers/src/vault/entries.rs`). |
| `--listen <address:port>` | Required: where the backend listens, which the proxy in front of it reaches ([HTTPS](#https)). |
| `--storage local\|s3` | Where the object store lives (`crates/backend-blobs/src/store.rs`). Default `local`. |
| `--s3-bucket`, `--s3-region`, `--s3-endpoint`, `--s3-force-path-style` | The bucket, with `s3`. The credentials come from `setup`'s own `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`, never from the command line, which other users of the machine can read. |
| `--cloud-data <directory>` | The machine manager's state directory, on one filesystem. Default `/opt/demi/data/cloud`. |
| `--cloud-limits on\|off` | Whether Clouds run under cgroup limits. Default `on`. `setup` refuses a machine without the cgroup v2 controllers unless the value is `off`. |
| `--version <version>` | The release to set up. Default the release of the running `demi-server`. |
| `--prerelease` | Set up the newest release, pre-releases included, instead of the running `demi-server`'s. Not with `--version`. |
| `--no-packages` | Install no system package: the operator installed e2fsprogs, bsdtar and nftables, on a distribution `setup` does not know ([Distributions](#distributions)). |
| `--no-input` | Never ask, as without a terminal ([A person or an agent](#a-person-or-an-agent)). |

A setting with no parameter keeps its default, as
`crates/backend/src/config/mod.rs` and
[Cloud setup](../cloud/setup.md#configuration) give them. An operator who needs
another adds it to `/opt/demi/config/demi.env` and restarts the services.

## HTTPS

Demi does not terminate TLS (`crates/backend/src/config/mod.rs`), and `setup`
sets up none either. The operator provides what serves the domain over HTTPS
and forwards to the backend, and tells `setup` where the backend listens. The
two usual arrangements differ only in that address:

- Behind a reverse proxy on the same machine, such as Caddy or nginx, or a
  tunnel, such as Cloudflare Tunnel: `--listen 127.0.0.1:<port>`.
- Behind Cloudflare's proxy with the SSL/TLS mode Flexible, which reaches the
  machine from outside over HTTP on port 80: `--listen 0.0.0.0:80`.
  - Another port needs an Origin Rule that sends the domain's requests to it.
  - The modes Full and Full (strict) reach the machine over HTTPS, which Demi
    does not serve.
  - The backend's unit lets it listen on a port below 1024.

`--help` gives both, and what the proxy must do: pass `Origin` and `Host`
unchanged and allow WebSocket upgrades.

The simplest arrangement uses Cloudflare's proxy, which needs no certificate
and no proxy on the machine. To set it up:

1. Manage the domain's DNS at Cloudflare and add an `A` record (and `AAAA`,
   for IPv6) that points the domain at the machine, with the proxy on, the
   orange cloud.
2. Under **SSL/TLS › Overview**, set the encryption mode to **Flexible**.
3. Install with `--listen 0.0.0.0:80`.

Cloudflare then serves the domain over HTTPS and reaches the machine over
plain HTTP on port 80, across the internet. For that leg to be encrypted too,
use Cloudflare Tunnel or a reverse proxy with its own certificate, such as
Caddy, which obtains one by itself, and `--listen 127.0.0.1:3271`.

## The steps

`setup` takes the following steps. It can repeat each step: running the same
command after a stop, such as for a proxy that was not ready yet, continues
where the last run stopped and repeats nothing that is done. Every failure says
what failed and what the operator does next, and exits with a status other
than 0.

1. **Check the machine.** `setup` requires root, Linux with systemd as its
   init, a distribution from [Distributions](#distributions), and no
   installation yet. It refuses a machine whose `/opt/demi/current` exists,
   and `demi-server upgrade` moves that machine.
2. **Install the system packages.** These are the packages the machine manager
   runs: e2fsprogs, bsdtar and nftables.
3. **Fetch the release.** `setup` fetches the release into
   `/opt/demi/releases/<version>`, as an upgrade does
   ([Fetch](upgrades.md#fetch)), and the gVisor version its manager pins into
   `/opt/demi/gvisor/<version>/` (`crates/server/src/gvisor.rs`).
4. **Check that the machine can run Cloud.** `setup` runs the release's own
   manager with `demi-machine-manager --check-host`. That command runs
   everything the manager's start checks before it serves
   (`crates/machine-manager/src/main.rs`), on the state directory and with the
   limits the parameters give, without starting anything. `setup` refuses a
   machine that fails, with the manager's reasons, such as a missing cgroup
   controller, or SELinux refusing gVisor.
5. **Write the installation.** `setup` creates the system user `demi` and the
   group `demi-cloud`, the data directories, and `/opt/demi/config/demi.env`
   readable by root alone. A configuration file that is already there and says
   something else than the parameters stops the run: `setup` never edits a
   configuration it did not just write.
6. **Start the release.** `setup` points `current` at the release, installs
   the units, enables and starts the services in order, and waits until each
   reports ready, as an upgrade's switch does
   ([Switch](upgrades.md#switch)).
7. **Check from outside.** An HTTPS request to the public URL answers with a
   valid certificate, and one naming another origin answers 403
   `forbidden_origin`, which shows the proxy passes `Origin`
   (`crates/backend-http/src/gate.rs`). A proxy that is not set up yet fails
   this step with what it must do. The run stops with the server running, and
   the same command checks again after the proxy is there.

## Distributions

`setup` reads `/etc/os-release` and installs the packages with the
distribution's own package manager. It recognizes a family by `ID` or `ID_LIKE`
and checks no version. The versions in the following table are the ones it is
meant for, and the Cloud check of step 4 decides whether an older machine can
run Demi.

| Distribution | Package manager | bsdtar's package |
| --- | --- | --- |
| Ubuntu 22.04 and later, Debian 12 and later | apt | `libarchive-tools` |
| Fedora, RHEL 10 and its rebuilds Rocky Linux and AlmaLinux | dnf | `bsdtar` |
| Arch Linux | pacman | `libarchive` |

e2fsprogs and nftables have those names everywhere. Four notes apply:

- **Another distribution.** `setup` names the three tools and stops. An
  operator who installed them runs it again with `--no-packages`, and the Cloud
  check of step 4 decides whether the machine can run Demi.
- **SELinux.** In enforcing mode, as the dnf distributions run it, SELinux lets
  gVisor run with no change of policy. `setup` passed its Cloud check this way
  on Fedora 44, Rocky Linux 10.2 and AlmaLinux 10.2, and a Cloud booted and ran
  commands on Fedora 44 and Rocky Linux 10.2.
- **Arch.** Its package names come from its repositories. `setup` has not run
  on an Arch machine yet.
- **openSUSE.** `setup` does not offer it.

Each distribution offered needs Linux 5.14 or later, systemd and cgroup v2,
which their current releases have.
