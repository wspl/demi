# Installation

A Demi server is installed on a Linux machine with one command, by a person
or by an AI agent: it puts `demi-server` on the machine and goes on into its
`setup`, which sets the server up. Every choice is a parameter of `setup`.
A person at a terminal is asked for what is missing; an agent, which runs
commands without a terminal, gets the guide to the parameters, `setup
--help`, and runs `setup` again with them. It installs the layout that
[Upgrades](upgrades.md#one-release-on-a-server) defines, and the server
moves between releases with `demi-server upgrade` from then on.

For example, an agent installs Demi on a fresh Ubuntu server, behind a
reverse proxy on the same machine that serves `demi.example.com` over HTTPS
and forwards to port 3271:

```sh
curl -fsSL https://github.com/wspl/demi/releases/latest/download/install.sh | sudo bash
# no terminal: prints the guide; the agent reads it and runs
sudo demi-server setup --domain demi.example.com --mode isolated --listen 127.0.0.1:3271
```

Run by a person in a terminal, the first command asks for the domain, the
mode and the listening address instead, and sets the server up with the
answers.

`setup` checks the machine, installs the few system packages the
machine manager needs, fetches the newest release, writes the configuration,
starts the backend and the machine manager, and checks the server from
outside. It ends by printing `https://demi.example.com`, where the first
visitor creates the master account on the setup page
([Authentication](../product/web-application.md#authentication)). Until it
exists anyone who reaches the domain can create it, so the operator opens the
page right away.

Installation is Linux only, on the distributions below, and a machine that
cannot run Cloud is refused: every deployment has Cloud. There is no
uninstall.

## The bootstrap

`install.sh` is an asset of every release, beside `demi-server-<target>` for
the two Linux targets ([Release workflow](builds-and-releases.md#release-workflow)).
It takes no parameter and does only what a shell must: it checks that it
runs as root on Linux, picks the target from the machine's architecture,
downloads that `demi-server` and the release's `SHA256SUMS` from the release
it came from, checks the one against the other, installs it as
`/usr/local/bin/demi-server`, and runs `demi-server setup` with the
parameters it was given (`bash -s -- <parameters>`), its input the session's
terminal, `/dev/tty`, when there is one, since its own input is the script.
On a machine that has a Demi server already, it changes nothing and points
to `demi-server upgrade`.

Everything else is `demi-server`'s, the program that upgrades the server
later, so setting up and upgrading share the fetch, the layout and the
units: a setup is the move from nothing to the first release, the release of
the `demi-server` that runs it unless `--version` names another. Once it has
started that release, `/usr/local/bin/demi-server` becomes a link to
`/opt/demi/current/bin/demi-server`, so the server's program on the path is
always the current release's.

## A person or an agent

No program can tell a person from an agent, but whether a terminal is there
to answer comes close, and it is what `setup` decides by:

| `setup` runs | With every required parameter | Without |
| --- | --- | --- |
| With a terminal | Sets the server up, asking nothing | Asks for each missing parameter, offering the defaults of the optional ones, then sets the server up |
| Without a terminal, as an agent runs it | Sets the server up | Prints the guide, `--help`, and exits with 0: the guide is the answer, not a failure |

`--no-input` makes a run with a terminal behave as one without. An agent
that already knows the parameters passes them to the first command, as
`bash -s -- --domain …`, and `setup` runs at once.

## Parameters

`demi-server setup --help` lists every parameter with its meaning, which
parameters each HTTPS choice needs, and complete examples; an agent reads it
and composes the command.

| Parameter | Meaning |
| --- | --- |
| `--domain <name>` | Required. The domain the product is reached at; the public URL is `https://<name>`. An IP address is refused ([Public URL and listening address](../backend/backend.md#public-url-and-listening-address)). |
| `--mode shared\|isolated` | Required, with no default ([Instance mode](../product/product.md#instance-mode-shared-vs-isolated)). |
| `--listen <address:port>` | Required: where the backend listens, which the proxy in front of it reaches ([HTTPS](#https)). |
| `--storage local\|s3` | Where the object store lives ([The object store](../backend/storage.md#the-object-store)). Default `local`. |
| `--s3-bucket`, `--s3-region`, `--s3-endpoint`, `--s3-force-path-style` | The bucket, with `s3`. The credentials come from `setup`'s own `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`, never from the command line, which other users of the machine can read. |
| `--expose-domain <name>` | The domain of expose hostnames ([Host expose](../execution/expose.md#deployment)). Optional: without it, exposes are off. |
| `--cloud-data <directory>` | The machine manager's state directory, on one filesystem. Default `/opt/demi/data/cloud`. |
| `--cloud-limits on\|off` | Whether Clouds run under cgroup limits. Default `on`; a machine without the cgroup v2 controllers is refused unless it is `off`. |
| `--version <version>` | The release to set up. Default the release of the running `demi-server`. |
| `--no-packages` | Install no system package: the operator installed e2fsprogs, bsdtar and nftables, on a distribution `setup` does not know ([Distributions](#distributions)). |
| `--no-input` | Never ask, as without a terminal ([A person or an agent](#a-person-or-an-agent)). |

A setting with no parameter keeps its default, as
[Configuration](../backend/backend.md#configuration) and
[Cloud setup](../cloud/setup.md#configuration) give them; an operator who
needs another adds it to `/opt/demi/config/demi.env` and restarts the services.

## HTTPS

Demi does not terminate TLS ([Public URL and listening address](../backend/backend.md#public-url-and-listening-address)),
and `setup` sets up none either: the operator provides what serves the
domain over HTTPS and forwards to the backend, and tells `setup` where
the backend listens. The two usual arrangements differ only in that address:

- Behind a reverse proxy on the same machine, such as Caddy or nginx, or a
  tunnel, such as Cloudflare Tunnel: `--listen 127.0.0.1:<port>`.
- Behind Cloudflare's proxy with the SSL/TLS mode Flexible, which reaches the
  machine from outside over HTTP on port 80: `--listen 0.0.0.0:80`. Another
  port needs an Origin Rule that sends the domain's requests to it; the modes
  Full and Full (strict) reach the machine over HTTPS, which Demi does not
  serve. The backend's unit lets it listen on a port below 1024.

`--help` gives both, and what the proxy must do: pass `Origin` and `Host`
unchanged and allow WebSocket upgrades. An expose domain needs the same proxy
to serve `*.<expose domain>` with a wildcard certificate; Cloudflare's free
certificate covers one level of wildcard below a zone, so behind Cloudflare
the expose domain is a zone of its own or the zone itself.

## The steps

These are the steps of `setup`. Each can be taken again: running the same
command after a stop, such as
for a proxy that was not ready yet, continues where the last run stopped and
repeats nothing that is done. Every failure says what failed and what the
operator does next, and exits with a status other than 0.

1. **Check the machine.** Root, Linux with systemd as its init, a
   distribution of the table below, and no installation yet: a machine whose
   `/opt/demi/current` exists is refused, and `demi-server upgrade` moves it.
2. **Install the system packages** the machine manager runs: e2fsprogs,
   bsdtar and nftables. A new machine often runs its distribution's
   automatic updates in its first minutes, which hold the package manager's
   lock: on hel1, Ubuntu's unattended upgrades held it for several minutes
   and a `setup` that did not wait failed at once. `setup` waits for the lock
   up to ten minutes, saying that it waits and for which process, and fails
   after that with the process that still holds it.
3. **Fetch the release** into `/opt/demi/releases/<version>`, as an upgrade
   does ([Fetch](upgrades.md#fetch)), and the gVisor version its manager
   pins into `/opt/demi/gvisor/<version>/`
   ([gVisor runtime](builds-and-releases.md#gvisor-runtime)).
4. **Check that the machine can run Cloud**, with the release's own manager,
   `demi-machine-manager --check-host`: everything its start checks before it
   serves ([Startup and recovery](../cloud/managed-hosts.md#startup-and-recovery)),
   on the state directory and with the limits the parameters give, without
   starting anything. A machine that fails is refused with the manager's
   reasons, such as a container that drops `CAP_SYS_RESOURCE`, or SELinux
   refusing gVisor.
5. **Write the installation**: the system user `demi` and the group
   `demi-cloud`, the data directories, and `/opt/demi/config/demi.env` readable by
   root alone. A configuration file that
   is already there and says something else than the parameters stops the
   run: `setup` never edits a configuration it did not just write.
6. **Start the release**: point `current` at it, install the units, enable
   and start the services in order, and wait until each reports ready, as an
   upgrade's switch does ([Switch](upgrades.md#switch)).
7. **Check from outside**: an HTTPS request to the public URL answers with a
   valid certificate, and one naming another origin answers 403
   `forbidden_origin`, which shows the proxy passes `Origin`
   ([Authentication and ownership](../backend/backend.md#authentication-and-ownership)).
   A proxy that is not set up yet fails this step with what it must do; the
   run stops with the server running, and the same command checks again once
   the proxy is there. With an expose domain, a random name under it is
   checked the same way.

## Distributions

`setup` reads `/etc/os-release` and installs the packages with the
distribution's own package manager:

| Distribution | Package manager | bsdtar's package |
| --- | --- | --- |
| Ubuntu 22.04 and later, Debian 12 and later | apt | `libarchive-tools` |
| Fedora, RHEL 10 and its rebuilds Rocky Linux and AlmaLinux | dnf | `bsdtar` |
| Arch Linux | pacman | `libarchive` |

e2fsprogs and nftables have those names everywhere. On another distribution
`setup` names the three tools and stops; an operator who installed
them runs it again with `--no-packages`, and the Cloud check of step 4
decides whether the machine can run Demi. SELinux in enforcing mode, as the
dnf distributions run it, lets gVisor run with no change of policy: `setup`
passed its Cloud check on Fedora 44, Rocky Linux 10.2 and AlmaLinux 10.2 so,
and a Cloud booted and ran commands on Fedora 44 and Rocky Linux 10.2. Arch's
package names come from its repositories; `setup` has not run on an Arch
machine yet. openSUSE is not offered. Each distribution offered needs Linux 5.14 or later, systemd
and cgroup v2, which their current releases have.

## Acceptance

`install.sh` and `setup` run end to end on a fresh Ubuntu machine, behind a
reverse proxy the test sets up, and the server serves the setup page at its
domain with a valid certificate, with the local store and with `s3`.
A second `setup` of a finished server is refused, one that stopped before
the proxy was ready continues, and the server then moves with `demi-server
upgrade` as any other.
