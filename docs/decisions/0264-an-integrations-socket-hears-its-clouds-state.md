---
status: proposed
---

# An integration's socket hears its Clouds' state

## Problem

A chat bot keeps a Cloud always on, and starts a long-running watcher on it
with `POST /v1/clouds/:cloud/commands`. A day later the Cloud reaches its
lifetime cap: Demi stops it, saves it and starts it again. The watcher is
gone. The program hears the command end as lost, but not that the Cloud
restarted, and it hears nothing of a Cloud that stopped for being idle or
was reset. To learn any of it, it has to poll `GET /v1/clouds`.

## Decision

The socket sends an event each time one of the integration's Clouds
changes state:

```json
{"type": "cloud", "cloud": "c1f0…", "state": "saving"}
```

`state` is the Cloud's state as `GET /v1/clouds/:cloud` gives it: `off`,
`booting`, `running`, `saving` or `resetting`. A restart at the lifetime cap
reads as `saving`, `off`, `booting` and `running`, in that order, and every
change is sent, however quickly the next follows. A boot that fails reads as
`booting` and then `off`, as a stop does; the event says what the state is,
not why. A program that missed events reads `behind` and reconciles from
`GET /v1/clouds`, as it does for conversations.

```rust
pub enum Event {
    // …
    /// One of the integration's Clouds changed state.
    Cloud { cloud: DeviceId, state: CloudState },
}
```

### Design

- The Cloud's machine already tells the user's pages when a change of its
  phase changes its state. It now also tells its shard, with the state.
- The shard queues each change and sends them in order: for each, it reads
  which integration the Cloud is of, and publishes the event to that
  integration's sockets. A Cloud of no integration, the user's own, is
  skipped. A shard with no socket open queues nothing.

## Spike

None.

## Prior art

- Fly.io's Machines API keeps each state change of a machine (`starting`,
  `started`, `stopping`, `stopped`) as an event on the machine, and lets a
  client wait for a state
  ([Machines API](https://fly.io/docs/machines/api/machines-resource/)).
- Kubernetes watches give every change of an object's status in order, with
  a full re-list when the watch falls behind; Demi's `behind` and
  `GET /v1/clouds` are the same pair.

## Alternatives

- The whole Cloud, as `GET /v1/clouds/:cloud` reads it, in each event, as
  the `conversation` event carries the whole conversation. It holds the init
  script's output, up to 16 MiB, which every change would send again.
- The latest state only, read when the event is sent. Changes that follow
  each other quickly would collapse into one: a restart could read as
  `running` to `running`, which tells nothing.

## Open questions

None.
