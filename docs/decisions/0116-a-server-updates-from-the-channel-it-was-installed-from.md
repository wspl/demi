---
status: proposed
---

# A server updates from the channel it was installed from

Supersedes: 0109 (A server takes whole releases of the channel it names),
the part that keeps Settings › Updates on Beta however the server was
installed.

## Problem

An operator installs Demi with `install.sh --canary` to try the newest
builds. Settings › Updates then says *Update channel: Beta* and *Demi is up
to date*, although newer Canary builds exist: Beta offers only the releases
Demi publishes, and Demi has published none. The operator learns about the
channel only by opening the menu, and a developer's backend started with
`xtask dev` shows Beta too, though it runs a build no Beta has.

## Decision

Settings › Updates starts on the channel the server was installed from, and
an administrator's choice there replaces it, as today.

```text
install.sh --canary          ->  Update channel: Canary
install.sh --beta            ->  Update channel: Beta
install.sh --version 0.1.22  ->  Beta when 0.1.22 is a published release,
                                 Canary when it is a pre-release
```

* `demi-server setup` writes the channel into the configuration it writes,
  `DEMI_UPGRADE_CHANNEL=beta|canary` in `/opt/demi/config/demi.env`, beside
  the other settings of the installation. `install.sh` passes its `--beta`
  or `--canary` on to `setup`, whose own `--beta` and `--canary` take the
  same newest release and name the channel; every released `setup` accepts
  them, so the script still installs a release older than itself. A run
  without a terminal that prints the guide ends with the command to run
  again, its own parameters, the channel among them, followed by the
  missing ones, so an agent's second run keeps the channel. With only a
  version, it is Canary when GitHub marks that release a
  pre-release and Beta otherwise; with `--from`, a developer's build, it is
  Canary.
* The backend takes the variable as the channel while `server_settings`
  holds none (`ControlService::upgrade_channel`); an administrator's choice
  writes the row, which wins from then on. A configuration without the
  variable is Beta, as a hand-written configuration is.
* `xtask dev` with `--offer-updates` sets `DEMI_UPGRADE_CHANNEL=canary`: a
  development build is a pre-release.
* No release edits the configuration, so an existing server keeps what it
  shows; its administrator changes the channel in Settings, as today.

```rust
/// `DEMI_UPGRADE_CHANNEL`: the channel the installation came from, which
/// Settings › Updates shows until an administrator chooses another.
#[arg(long, env = "DEMI_UPGRADE_CHANNEL", value_enum, default_value = "beta")]
pub upgrade_channel: UpgradeChannel,
```

## Spike

None: setup already resolves the channel or the version against GitHub's
releases, and the backend already reads the row.

## Prior art

* **Firefox** records its update channel, `app.update.channel`, in the
  installation itself rather than in the profile: a Beta or Nightly
  installation updates from its own channel.
* **Chrome** and **VS Code** ship one installer per channel (Beta,
  Insiders), and an installation updates from the channel of its installer.

Demi takes the same rule, with one installer that records the channel it
was asked for, and keeps the administrator's switch, which Firefox has only
through its preferences.

## Alternatives

* **Derive the channel from the running release:** Canary while the server
  runs a pre-release. It needs no setting, but the channel would change by
  itself the day the running build is published, and it is unknown until
  the backend first reaches GitHub.
* **Ask for the channel in setup:** one more question, whose answer the
  command line already gave.

## Open questions

None.
