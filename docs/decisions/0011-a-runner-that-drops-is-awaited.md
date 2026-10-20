---
status: accepted
---

# A runner whose connection drops is awaited before its device is offline

## Context and Problem Statement

A runner reconnects by itself when its connection drops, after 250 ms at
first. Most drops are short: a connection reset on the way to the backend
is usually back within two seconds. The backend, however, marks the device
offline the moment the connection ends. The conversation then shows the
offline notice above the composer, and an operation that needs the device
during the gap fails as offline, although the runner is back a moment later.

Only the backend's own restart is tolerated today: an operation waits up to
30 seconds after the start for a runner the shutdown disconnected, while the
page still shows the device offline.

## Considered Options

* A dropped connection starts a wait for the runner; the device counts as
  online until the wait ends without it.
* The page delays the offline notice, and the backend stays as it is.

## Decision Outcome

Chosen option: the backend awaits the runner, because a delayed notice
would hide the drop while operations still failed during it. Editors with
remote connections, such as VS Code's, reconnect quietly the same way.

* When a device's connection ends other than by revocation or by the user
  disconnecting the device, the device is returning for 10 seconds.
* While a device is returning:
  * It counts as online everywhere the user sees it, and the offline notice
    does not show.
  * An operation that needs the device waits for the runner for the rest of
    the 10 seconds, and goes on once the runner is back.
* If the runner is not back when the 10 seconds end, the device is offline:
  the notice shows, and the operations still waiting fail as offline.
* The backend's restart is the same wait, with the 30 seconds it keeps from
  its start, so the device no longer shows offline during that wait either.
* Commands running on the runner are unaffected, as today: the runner keeps
  them across connections.

```text
 connection drops ──▶ returning (10 s; 30 s from a backend start)
                        │  shown online, no notice
                        │  new operations wait for the runner
                        ├── runner back ──▶ online; waiting operations go on
                        └── time is up  ──▶ offline; the notice shows and
                                            waiting operations fail
```

### Consequences

* Good, because a drop the runner recovers from on its own is no longer
  shown, and nothing the user started during it fails.
* Bad, because a device that has really gone away shows offline 10 seconds
  later than today, and an operation started then waits those 10 seconds
  before it fails.
