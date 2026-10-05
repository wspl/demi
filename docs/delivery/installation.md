# Installation

A Demi server is installed with one command on a Linux machine, by a person
or by an AI agent. Every choice is a parameter, `--help` is the complete
guide to them, and the installer asks only when a required parameter is
missing and a terminal is there to answer. It installs the layout that
[Upgrades](upgrades.md#one-release-on-a-server) defines, and the server
moves between releases with `demi-server upgrade` from then on.

For example, an agent installs Demi on a fresh Ubuntu server, behind a
reverse proxy on the same machine that serves `demi.example.com` over HTTPS
and forwards to port 3271:

```sh
curl -fsSL https://github.com/wspl/demi/releases/latest/download/install.sh \
  | sudo bash -s -- --domain demi.example.com --mode isolated --listen 127.0.0.1:3271
```

The installer checks the machine, installs the few system packages the
machine manager needs, fetches the newest release, writes the configuration,
starts the backend and the machine manager, and checks the server from
outside. It ends by printing `https://demi.example.com`, where the first
visitor creates the master account on the setup page.

Installation is Linux only, on the distributions below, and a machine that
cannot run Cloud is refused: every deployment has Cloud. There is no
uninstall.

## The bootstrap

`install.sh` is an asset of every release, beside `demi-server-<target>` for
the two Linux targets ([Release workflow](builds-and-releases.md#release-workflow)).
It does only what a shell must: it checks that it runs as root on Linux,
picks the target from the machine's architecture, downloads that
`demi-server` and the release's `SHA256SUMS` from the release it came from,
checks the one against the other, and runs `demi-server install` with its own
arguments. Everything else is `demi-server`'s, the program that upgrades the
server later, so the two share the fetch, the layout and the units: an
installation is the move from nothing to the first release.

After the installation, `/usr/local/bin/demi-server` links to
`/opt/demi/current/bin/demi-server`, so the server's own program is on the
path.

## Parameters

`demi-server install --help` lists every parameter with its meaning, which
parameters each HTTPS choice needs, and complete examples; an agent reads it
and composes the command.

| Parameter | Meaning |
| --- | --- |
| `--domain <name>` | Required. The domain the product is reached at; the public URL is `https://<name>`. An IP address is refused ([Public URL and listening address](../backend/backend.md#public-url-and-listening-address)). |
| `--mode shared\|isolated` | Required, with no default ([Instance mode](../product/product.md#instance-mode-shared-vs-isolated)). |
| `--listen <address:port>` | Required: where the backend listens, which the proxy in front of it reaches ([HTTPS](#https)). |
| `--storage local\|s3` | Where the object store lives ([The object store](../backend/storage.md#the-object-store)). Default `local`. |
| `--s3-bucket`, `--s3-region`, `--s3-endpoint`, `--s3-force-path-style` | The bucket, with `s3`. The credentials come from the installer's own `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`, never from the command line, which other users of the machine can read. |
| `--expose-domain <name>` | The domain of expose hostnames ([Host expose](../execution/expose.md#deployment)). Optional: without it, exposes are off. |
| `--cloud-data <directory>` | The machine manager's state directory, on one filesystem. Default `/var/lib/demi/machine-manager`. |
| `--cloud-limits on\|off` | Whether Clouds run under cgroup limits. Default `on`; a machine without the cgroup v2 controllers is refused unless it is `off`. |
| `--version <version>` | The release to install. Default the newest. |
| `--no-packages` | Install no system package: the operator installed e2fsprogs, bsdtar and nftables, on a distribution the installer does not know ([Distributions](#distributions)). |
| `--no-input` | Never ask: a missing required parameter fails with its name. Without a terminal, the installer never asks anyway. |

A setting with no parameter keeps its default, as
[Configuration](../backend/backend.md#configuration) and
[Cloud setup](../cloud/setup.md#configuration) give them; an operator who
needs another adds it to `/etc/demi/demi.env` and restarts the services.

## HTTPS

Demi does not terminate TLS ([Public URL and listening address](../backend/backend.md#public-url-and-listening-address)),
and the installer sets up none either: the operator provides what serves the
domain over HTTPS and forwards to the backend, and tells the installer where
the backend listens. The two usual arrangements differ only in that address:

- Behind a reverse proxy on the same machine, such as Caddy or nginx, or a
  tunnel, such as Cloudflare Tunnel: `--listen 127.0.0.1:<port>`.
- Behind Cloudflare's proxy, which reaches the machine from outside:
  `--listen 0.0.0.0:<port>`, with one of the ports Cloudflare forwards HTTP
  to, such as 8080.

`--help` gives both, and what the proxy must do: pass `Origin` and `Host`
unchanged and allow WebSocket upgrades. An expose domain needs the same proxy
to serve `*.<expose domain>` with a wildcard certificate; Cloudflare's free
certificate covers one level of wildcard below a zone, so behind Cloudflare
the expose domain is a zone of its own or the zone itself.

## The steps

Each step can be taken again: running the same command after a stop, such as
for a proxy that was not ready yet, continues where the last run stopped and
repeats nothing that is done. Every failure says what failed and what the
operator does next, and exits with a status other than 0.

1. **Check the machine.** Root, Linux with systemd as its init, a
   distribution of the table below, and no installation yet: a machine whose
   `/opt/demi/current` exists is refused, and `demi-server upgrade` moves it.
2. **Install the system packages** the machine manager runs: e2fsprogs,
   bsdtar and nftables.
3. **Fetch the release** into `/opt/demi/releases/<version>`, as an upgrade
   does ([Fetch](upgrades.md#fetch)).
4. **Check that the machine can run Cloud**, with the release's own manager,
   `demi-machine-manager --check-host`: everything its start checks before it
   serves ([Startup and recovery](../cloud/managed-hosts.md#startup-and-recovery)),
   on the state directory and with the limits the parameters give, without
   starting anything. A machine that fails is refused with the manager's
   reasons, such as a container that drops `CAP_SYS_RESOURCE`, or SELinux
   refusing gVisor.
5. **Write the installation**: the system user `demi` and the group
   `demi-cloud`, the data directories, and `/etc/demi/demi.env` readable by
   root alone. A configuration file that
   is already there and says something else than the parameters stops the
   run: the installer never edits a configuration it did not just write.
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

The installer reads `/etc/os-release` and installs the packages with the
distribution's own package manager:

| Distribution | Package manager | bsdtar's package |
| --- | --- | --- |
| Ubuntu 22.04 and later, Debian 12 and later | apt | `libarchive-tools` |
| Fedora, RHEL 9 and later, Rocky Linux, AlmaLinux | dnf | `bsdtar` |
| openSUSE Leap and Tumbleweed | zypper | `bsdtar` |
| Arch Linux | pacman | `libarchive` |

e2fsprogs and nftables have those names everywhere. On another distribution
the installer names the three tools and stops; an operator who installed
them runs it again with `--no-packages`. Each of these needs Linux 5.14 or
later, systemd and cgroup v2, which their current releases have; the Cloud
check of step 4 is what decides.

## Acceptance

The installer runs end to end on a fresh machine of each package manager,
behind a reverse proxy the test sets up, and the server serves the setup page
at its domain with a valid certificate; on Ubuntu also with `s3`. A second run of a
finished installation is refused, one after DNS was missing continues, and
the installed server then moves with `demi-server upgrade` as any other.
