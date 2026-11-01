---
status: proposed
---

# Downloading updates ahead is a choice, off by default

Supersedes: 0125 (A server prepares its next release ahead), the part that
offers nothing to turn the work ahead off.

## Problem

Alex runs a server on Canary, which publishes every push to main that
passes, several releases a day. The server checks every hour, and each
check that finds a newer release downloads all of it in the background:
the release, its gVisor and its Cloud image, and every paired device
downloads its runner. Alex updates once a week, so six days of those
downloads are thrown away by the next release, and a server whose host
counts traffic pays for each of them.

## Decision

Settings › Updates gets a switch below the channel, off on every server,
the ones that update to this release included.

```text
┌──────────────────────────────────────────────────────────────────────┐
│ Update channel                                         [ Canary ⌄ ]  │
│ Canary gets the newest builds first, before anyone has tried them on │
│ a server.                                                            │
├──────────────────────────────────────────────────────────────────────┤
│ Download updates automatically                               ( ○ )   │
│ Not recommended on Canary: it uses a lot of data.                    │
└──────────────────────────────────────────────────────────────────────┘
```

- **Off.** The checks keep their cadence, every hour on Canary and every
  day on Beta, and a newer release shows as *Demi 0.1.5500 is available*
  with **Update Now** and *You have 0.1.5469.* Nothing is downloaded until
  Alex clicks **Update Now**, which lists its seven steps from the
  download, as before the work ahead existed. The devices download their
  runner then too.
- **On.** The server gets the newest release ready as soon as a check
  finds it, as it does today. Turned on while a newer release is known, it
  starts at once, without waiting for the next check.
- **Turned off while the work ahead runs.** That work finishes: it runs as
  root in `demi-prepare.service`, which the backend cannot stop. A request
  for a newer release that waits behind it is withdrawn, so no new work
  starts after it. A release already got ready stays, and Update Now uses
  it.
- Only the master and administrators see the switch and change it, as the
  channel.

### How it works

- `server_settings` gets `prepare_ahead INTEGER NOT NULL DEFAULT 0`, 0 or
  1. A server that never changed a setting has no row, which reads as off.
- `ServerUpgrade` gets `prepare_ahead: bool`, and `PUT
  /server-upgrade/prepare-ahead` takes `SetPrepareAhead { prepare_ahead:
  bool }`.
- `Upgrades` holds the setting with the channel. A check writes the
  prepare request only when it is on; turning it on writes the request for
  the newer release the last check found, unless an update runs; turning
  it off removes a request `demi-server` has not taken yet. A turn of the
  switch waits for a check that runs, as a change of channel does, so the
  two never write the request at once.

### What an older page or data meets

A page of an earlier release reads `ServerUpgrade` without the field and
shows no switch, until it loads the new one. The migration adds the column
with its default, so every server starts off.

## Spike

None: the switch decides only whether the existing prepare request is
written.

## Prior art

- macOS Software Update has *Download new updates when available*, on by
  default.
- Windows Update has *Get the latest updates as soon as they’re
  available*, and a *metered connection* setting that stops downloads
  until the user asks.
- [Home Assistant](https://github.com/home-assistant/supervisor) checks for
  new versions on a schedule and downloads only when the user installs.

Demi takes macOS's switch and Home Assistant's default: a channel that
publishes several times a day makes downloading each release by default
cost more than the minute Update Now saves.

## Alternatives

- **Off on Canary only, with no switch.** A Canary server on a host with
  plenty of traffic loses the quick update with no way to have it back.
- **Download ahead at most once a day.** Still pays for releases no one
  installs, and adds a rule the screen cannot explain.

## Open questions

None.
