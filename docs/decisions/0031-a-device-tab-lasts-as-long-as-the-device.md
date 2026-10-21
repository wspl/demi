---
status: accepted
---

# A device's tab lasts as long as the device

Supersedes, in part: 0017 (each device streams on its own), where a device
nobody looks at closes as idle.

## Context and Problem Statement

A conversation's simulators and emulators show as tabs in the work panel. The
Host's mobile service keeps which conversation has which device open, under
which handle, such as `m1`, in its memory only. When the runner updates or
restarts, that service starts again with an empty list, and the backend
removes every device tab of every conversation, though each simulator is
still running on the Host. The agent's handles stop working too.

The service also shuts down a device Demi booted after an hour in which
nobody watched it and no command used it, and closes its tab. A user who
comes back after a meeting finds the tab gone.

## Considered Options

* Keep today's rules.
* Keep the tab through a restart, and keep the idle shutdown.
* A tab lasts as long as its device: no idle shutdown.

## Decision Outcome

Chosen option: a tab lasts as long as its device, because a connection that
drops and comes back, or time passing, says nothing about whether the user is
done with a device.

* A device tab closes only when the user or the agent closes it, when its
  conversation is released, or when its device no longer exists on the Host.
* Nothing closes or shuts down a device for being unused.
* When the runner restarts or updates, the tab stays and says the device is
  reconnecting. The Host's mobile service keeps which conversation has which
  device open, under which handle, across its restarts, as it keeps which
  simulators Demi booted. When it starts again it takes each device back
  under the same handle, so the tab's picture returns and the agent's handles
  work as before.
* A device that no longer exists when the service starts again is closed as
  lost, and its tab goes.
* A Host that stops, such as a Cloud machine, keeps its tabs; they reconnect
  when it starts.

### Performance

Estimated, not measured:

* A restart reads one small file per Host, as it already reads the record of
  booted simulators, and lists the Host's devices once.
* A device stays booted until someone closes it: a simulator or emulator left
  open keeps its memory, about 1 to 3 GB, and a little processor time, until
  the user or the agent closes its tab. Its stream sends nothing while nobody
  watches it (0017).

### Consequences

* Good, because an update or a short drop no longer costs the user their
  device tabs, nor the agent its handles.
* Good, because a device's life has one rule: it ends when someone ends it or
  it is gone.
* Bad, because devices nobody closes keep using the Host's memory; the user
  closes them from their tabs.
