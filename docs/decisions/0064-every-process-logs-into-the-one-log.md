---
status: proposed
---

# A command package's service and the machine manager write the one log's lines

## Problem

A user's browser tab stops answering. The runner's log has the browser
service's standard error as text lines, `event: "stderr"` and a sentence in
`fields.text`, with no event name, no conversation and no duration, so the
lines of that one conversation cannot be found among the others. A Cloud
whose sandbox stopped by itself is worse: the machine manager writes only to
the systemd journal of its server, and `log.sqlite` has nothing of why.

## Decision

Every process of Demi writes the lines 0060 defines, and they reach
`log.sqlite`.

- **A command package's service** (the `command` process of a `plugin-*`
  crate, and `command-package-claude-code`) writes each `tracing` event as
  one JSON `LogLine` on its standard error, through a layer of
  `command-sdk` that every service installs; a line longer than the
  runner's lines of standard error, 4096 bytes, has its longest text field
  shortened to fit. The runner keeps a line that reads as a `LogLine` with
  an event name as the line it is, with `origin: Runner`; any other line, as
  a panic's, is still kept as `stderr` text. A service's call carries its
  invocation and, when it serves a conversation, the conversation, which
  its lines name; a call carries no request ID.
- **The machine manager** writes its `tracing` events to the journal as
  today, and sends each, as a `LogLine` with `origin: MachineManager`, on
  the backend's connection when one is open. The backend stores them as it
  stores its own. A line made while no connection is open stays in the
  journal alone.

```rust
// machine-manager-protocol
pub enum MachineResponse {
    // ...
    /// A line of the manager's log, sent on every open connection as a
    /// death is.
    Log { line: LogLine },
}
pub const VERSION: u32 = 5; // was 4

// shared-types
pub enum LogOrigin {
    // ...
    MachineManager,
}

// shared-types, feature `tracing`: the layer that makes a `tracing` event
// a `LogLine`, which the services and the manager share.
pub struct LineLayer<W>;

// command-sdk: installed by every service's main, before it serves.
pub fn log_to_stderr(service: &'static str);
```

The backend and the manager of a deployment are one release, so the wire
version's change needs nothing else; a runner of an earlier release keeps
an earlier service's text lines as text.

## Spike

None.

## Prior art

- systemd's journal takes a service's standard error as its log and parses
  structured fields when the service writes them
  ([sd_journal_print](https://www.freedesktop.org/software/systemd/man/latest/sd_journal_print.html)).
- Kubernetes keeps a container's standard error as its log, and its
  structured logging writes JSON lines there
  ([structured logging](https://kubernetes.io/docs/concepts/cluster-administration/system-logs/#structured-logging)).

Demi does the same for its services, through the runner that already reads
their standard error.

## Alternatives

- **A log connection of its own for each service.** The runner reads a
  service's standard error already; a second channel adds a protocol and
  gains nothing.
- **The backend records only its side of the manager's calls.** It cannot
  say why a sandbox stopped by itself, which no call asked.

## Open questions

None.
