---
status: proposed
---

# An update waits for no device

Supersedes: 0111 (An update shows each step it takes, with its progress), the
step *Update Demi on your devices*; and 0117 (A runner update brings the
package programs), the part that says the upgrade's wait for the devices
covers the programs.

## Problem

An administrator updates a server with five paired devices from Settings ›
Updates. The preparation's last step, *Update Demi on your devices*, waits
until every device has downloaded the new runner, or for two minutes. One
laptop on a slow network keeps everyone waiting for those two minutes, and a
laptop that is asleep always costs the full two: no one can use Demi on the
new version until then, although the server is ready and the other devices
are too.

A device that has not downloaded the new runner already updates after the
restart: the new backend sends its runner the new executable, and the device
shows as updating until its runner is back. The wait only moves that time
from one device to everyone.

## Decision

The update restarts as soon as the server's own steps are done. Each device
updates on its own, before the restart when it can, after it when it must.

```text
┌──────────────────────────────────────────────────────────────────────┐
│ Updating to Demi 0.1.5500                       ███████████░░░░░░░░  │
│ Step 6 of 7 · Checking the databases                                 │
│                                                                      │
│ ✓ Download Demi 0.1.5500                          214 MB in 21 s     │
│ ✓ Unpack the release                              310 MB in 4 s      │
│ ✓ Check the configuration                         Done               │
│ – Download gVisor                                 Already installed  │
│ ✓ Install the Cloud image                         96 MB in 9 s       │
│ ◌ Check the databases                                                │
│ ○ Restart Demi                                                       │
└──────────────────────────────────────────────────────────────────────┘
```

- **The list loses the devices' step.** It has seven lines in every update.
- **The devices download ahead in the background.** The preparation tells the
  running backend which runner release comes next as its first step, and the
  connected devices download it, with their programs, while the server checks
  its configuration, installs the image and checks the databases. Nothing
  waits for them, and nothing shows their downloads on the update's row.
- **A device that is not done by the restart updates after it.** The new
  backend sends its runner the new executable when it connects, as it does
  today, and the device shows as updating, with its percent, until its runner
  is back on the new version. Meanwhile the device cannot be used: a command
  on it fails as on a device that is away, and works again once its runner is
  back. Every other device, and everything that needs none, works at once.

### How it works

- `demi-server` sends the prefetch request over the management socket on a
  thread of its own, at the start of the preparation, and reads the answers
  only to write the devices' counts to its output. The thread ends when the
  backend closes the socket as it stops, or when every device is done; the
  preparation never joins it, so the two-minute limit goes.
- `UpgradeStepKind::UpdateDevices`, `StepProgress::Devices` and
  `StepSummary::Devices` go, with the Settings row's line that said when the
  update goes on without the devices.
- The backend's side of the prefetch, the devices' downloads and their
  updating state after a restart stay as they are.

### What an older backend or page meets

The running backend during the preparation is of the release before. It
shows the record `demi-server` sends, which no longer names the devices'
step, as a list of seven lines; the step's kind it knows and does not meet
changes nothing.

## Spike

None: the prefetch and a device's update after the restart exist and are
unchanged; only when the preparation starts the one and stops waiting for it
changes.

## Prior art

- [Tailscale](https://github.com/tailscale/tailscale)'s coordination server
  never waits for clients: each node updates on its own schedule, and the
  server speaks to old and new nodes alike.
- VS Code updates its remote server when a client of the new version first
  connects, not when the client updates; the window waits for its server
  alone.

Demi's backend serves only the runner of its own release, so a device that has
not updated cannot run commands until it has, as VS Code's window cannot.

## Alternatives

- **A shorter limit.** It still holds every user for a device that is asleep.
- **Keep the line and stop waiting.** A line whose numbers no step waits for
  says the update depends on them, which it no longer does.

## Open questions

None.
