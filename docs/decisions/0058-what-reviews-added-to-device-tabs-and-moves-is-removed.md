---
status: accepted
---

# What reviews added to device tabs and moves is removed

Follows 0043 (what reviews added beyond their designs is removed), for the
two packages it left until the recording work had landed: 0031 (a device
tab lasts as long as the device) and 0027 (a move applies at once).

## Summary

Mechanisms that reviews added to device tabs and to moves, beyond what
their records ask, are removed. A user meets no change.

## Motivation

Each mechanism guards a case no user met and the acceptance of its package
could not reproduce, and each makes the code that every tab and every move
runs through longer and harder to change:

* 0031: a file lock across processes with a takeover and an owner in each
  kept device, so two device services of one runner, an old one and its
  successor, could not both drive a device; and a guard against a device's
  service stopping while another step on it runs.
* 0027: one "change boundary" that a move, an archive and a detach share,
  and a gate per device that a move waits on.

## Guide-level explanation

Nothing changes on screen. A device's tab still lasts as long as the
device: when the runner restarts or a device's stream drops, the tab shows
that it reconnects until the device is back, and closes only when the
device is gone.

## Reference-level explanation

* 0031's device tab: the lock file, its takeover, the `owner` of each kept
  device and the stop interleaving guard are removed. Taking a device back
  keeps retrying while the device is booting, while the Host cannot list
  its devices, and after an attach fails; the tab closes only when the
  Host lists its devices without it.
* 0027's move: the shared change boundary and the per-device gate are
  removed. An archive and a detach keep their hold on the conversation; a
  move takes no lock, as 0027 says. A move of an archived conversation is
  refused before it asks any Host, and writes its log line as every move
  does.
* A release to the Host a move left still waits for the shell jobs that
  ran there for the conversation. A Host that cannot be resolved gets no
  release.
* The tests that exist only for the removed mechanisms are removed with
  them.

## Upgrade and version skew

The kept devices file drops `owner`, which 0.1.46 to 0.1.56 wrote and
require. A runner of this version reads their file. A runner of one of
those versions, after a downgrade, cannot read the file this version
writes and discards it: the tabs open before the downgrade are not taken
back, and the devices Demi booted are no longer shut down by Demi. The old
lock file stays in the device service's directory, unused.

## Performance

Less code runs on each device listing and each move.

## Drawbacks

* When the backend restarts with a new device service and the runner does
  not, which only a development setup does, the old and the new service
  may both drive a device and overwrite each other's kept devices file.
* A move does not wait for a command that runs on the Host it left through
  `--host` or a one-off call; the release may end such a command's device
  service while it runs.
* A move that an archive or a detach races may skip one release, on a
  Host that holds nothing for the conversation.

## Rationale and alternatives

* Keep the mechanisms: each guards a case no user met, at the cost of the
  code every user's tabs and moves run through.

## Prior art

* 0043 removed the same kind of additions from seven other packages.

## Unresolved questions

None.
