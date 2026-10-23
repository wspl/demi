---
status: proposed
---

# Nothing a page asks for, and no step of a conversation, waits for a Host

Supersedes, in part: 0027 (a move applies at once), where a move asks the
destination's Host about the directory and fails when it does not answer.

## Problem

A user opens a conversation whose work panel keeps a browser tab. To send
the conversation's metadata, the backend asks the Host's browser package
for the conversation's tabs, inline in the conversation's socket, which
reads no frame from the page and sends none until the Host answers. Every
change of the metadata asks again, and a Host holding no tab for the
conversation is never remembered as holding none, so a busy conversation
asks dozens of times a minute, and an opening waits seconds while the Host
starts its browser service to answer. The same pattern is elsewhere:

- The sync channel reads the Cloud's disk sizes from the machine manager
  inline, with no deadline, and the backend does not start serving until
  the manager answers its recovery at startup.
- Archiving, detaching, deleting, an idle release, a turn's pending detach
  and deleting a project wait for each Host's release, up to 360 seconds a
  Host, one Host after another, while they hold the conversation's file
  gate, so a message the user sends meanwhile waits behind them; turning a
  plugin off waits in its request.
- Restoring a conversation with a command recorded as running admits the
  Host to take the command up: it waits up to 30 seconds for a Host that
  just dropped, and wakes a stopped Cloud and waits for it to start.
- Moving a conversation or creating one in a directory asks the Host
  whether the directory exists, up to 10 seconds, and on a Cloud wakes the
  machine with no deadline.
- A work panel read of a Host's files holds the conversation's file gate
  for the whole Host call, with no deadline, so a queued transition, and
  every message behind it, waits for it.

## Decision

A conversation opens, a subagent shows, a message sends, and an archive,
detach or delete finishes, in the time the backend takes, whether the Host
is near, slow, asleep or offline. What the work panel shows of a Host is
what the Host last reported; while the Host is away the panel says so and
keeps it. Only a request the user makes of a Host waits for it, alone:
reading a file, a live view, binding a browser, a
device's directory dialog.

| Path | Waits for a Host? |
|---|---|
| A page's reads and its sockets, the sync channel included | Never |
| Opening, restoring, sending, a turn's start and end | Never |
| A transition: archive, detach, delete, idle release, plugin off, project delete | Never; its Hosts are told, and settle at their next hello |
| Moving or creating a conversation in a directory | Never; the Host creates the directory at its first use |
| A request the user makes of a Host | Yes, alone: no conversation gate |
| A turn's own work: its tools, a provider that runs a process | Yes, as the turn's work |

**The Host reports, the backend keeps.** The browser package reports a
conversation's tabs whole when they change, where it now reports only that
they changed, and an empty list too:

```rust
// plugin-browser protocol
pub struct BrowserMessage {
    pub tabs: TabsChange,
}
pub enum TabsChange {
    /// The conversation's tabs on this Host, whole, at `revision`, and the
    /// ones closed on purpose since the last report, which the panel drops
    /// where a tab lost with the browser stays.
    Tabs { revision: u64, tabs: Vec<BrowserTab>, closed: Vec<TabId> },
}
```

```sql
-- control database: what a plugin's package last reported of a conversation
CREATE TABLE plugin_reports (
  plugin_id       TEXT NOT NULL,
  conversation_id TEXT NOT NULL REFERENCES conversations (id) ON DELETE CASCADE,
  device_id       TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
  revision        INTEGER NOT NULL,
  value           TEXT NOT NULL,     -- the browser's: Vec<BrowserTab>
  reported_at     INTEGER NOT NULL,
  PRIMARY KEY (plugin_id, conversation_id, device_id)
) STRICT;
```

- The plugin's page state, `GET /conversations/:id/plugins/:plugin/state`
  included, answers the stored list, an empty one included, and never calls
  the Host.
- When a runner says hello it tells each package's latest report for each
  conversation again; a package that is lost drops its reports. A Host that
  has never reported shows the panel's tabs as last stored, with no live
  view until it does.

The machine manager pushes the Cloud's status when it changes, and the
backend keeps the latest; `GET /cloud`, `/api/state` and the sync channel
read it, and answer no volumes before the first:

```rust
// machine-manager protocol, manager to backend
pub enum ManagerEvent {
    CloudStatus {
        device_id: String,
        volumes: Option<CloudVolumes>,
        newer_image: bool,
    },
}
```

The manager sends each device's status when the backend connects and after
each operation on a device; the backend connects at once and again every
5 seconds while it cannot, and starts serving before the manager answers
its recovery at startup, which then runs in the background. The manager's
wire version is 6.

**A release is reconciled, not waited for.** A transition commits and
answers; it holds the file gate only for its commit. What a Host should
hold is derived from the backend's records, never kept as a queue of
releases. The backend's hello carries it:

```rust
// runner-protocol, HelloOk gains
/// The conversations whose services this device keeps: not archived, on
/// it as their Host or an attached one, and not released for idleness
/// since their last use, which the backend keeps in memory.
pub held: Vec<ConversationId>,
/// The packages of the plugins turned off that no plugin turned on uses.
/// A package of no plugin, such as a provider's, is never named.
pub stopped: Vec<PackageId>,
```

- The runner releases each conversation it has kept anything of that
  `held` does not name, and stops the packages `stopped` names. This alone
  sets a Host right, after it was offline, after a release failed, or
  after either end restarted. A backend that cannot read either list
  drops the connection, never sending an empty one.
- While the Host is connected, a transition also tells it at once; a
  message that fails or is lost changes nothing, as the next hello
  settles it.
- No release is stored and none is withdrawn: restoring a conversation,
  attaching it, moving it back or turning a plugin on again is the
  backend's records naming it again.
- Deleting a conversation stops its in-flight Host reads and transfers, as
  a look does today, instead of waiting for them.
- Where the backend ends a conversation's commands, deleting it, resetting
  a Cloud or ending a subagent, a command on a Host that is away ends at
  once as lost, "its Host was away when the work ended", instead of
  waiting for the Host to confirm its stop; the Host stops it when it
  comes back, as a job the backend no longer has.

**A restored conversation takes its commands up from the runner.** It
attaches each command as last recorded and admits no Host; the runner's
hello lists the jobs it runs and the backend takes each up from that
report, as at a reconnect. Restoring wakes no Cloud.

**Moving and creating.** `PATCH /conversations/:id { target }` and
creating a conversation store the directory as the user typed it, with `~`
expanded, and answer at once; a directory chosen in a picker is already
the Host's own spelling. The first time a conversation's work admits the
Host in that directory (a shell job, a `demi host` command, an upload or
download), the backend asks the runner once, on that
connection, to create the directory with its parents and name it:

```rust
// runner-protocol: the existing question gains a switch
FsDirectory { path: String, create: bool }
// answers the canonical path, or the reason it cannot be one
```

- The canonical path replaces the typed one, and the conversation's
  project becomes the one of that path: if a project of that path exists,
  the conversation joins it, and the typed one goes when nothing else is in
  it, as two spellings of one directory are never two projects.
- A directory that cannot be created fails that first use with a reason
  the runner words from the first part of the path that exists ("`x` is a
  file", "drive `Q:` does not exist"), since Windows answers "already
  exists" for a directory under a file. A mistyped directory is created,
  where the user sees it in the conversation's header and can move it.
- Reading a directory that does not exist yet, a work panel read, a look, a
  file watch, a live view, the project's instruction search, finds that it
  does not exist and creates nothing; the page shows the Host's reason,
  as wherever a Host could not do what the page asked; once the directory
  is created, its watches and instruction search start again.

**A request of a Host holds no conversation gate.** A work panel read
takes the gate only to resolve its Host and directory, and calls the Host
without it. A Host call has no deadline: it ends when the Host answers, or
when the link finds the Host offline, after 60 seconds with nothing heard.

**The loops that serve a page await only their channels.** Building a
frame of the conversation socket or the sync channel reads held values;
anything else is spawned and posts its result to the loop, whose `select!`
is biased towards the page's frames.

**Versions.** The runner's wire version rises for the browser package's
tabs and the directory's first use. A runner of an earlier version is
refused and updates itself, as at every release; the wire version is 61.
`plugin_reports` comes with a migration of the control database. After
the upgrade, a command on a stopped Cloud shows as running until that
Cloud next says hello, where restoring used to wake it.
Releases an earlier build left unanswered need nothing: the first hello
settles them.

Each step logs what it waited for: `host_call.ended` with its kind, `ms`
and outcome, and each release a hello or a transition asks for, with what
the runner answered.

## Spike

Where a conversation first uses its directory, read in the code, and a
probe doing the runner's creation and naming on macOS (APFS) and Windows:

| Point | What it showed |
|---|---|
| Who needs the directory to exist | Only a process started in it: a shell job or a `demi host` command fails without it (`CwdUnusable`). A plugin's service runs elsewhere and takes the directory as a string; reads and live views read under it |
| Creating with its parents | Works in one step and again, on both systems |
| Spelling | A differently cased existing `a/b` is reused, not duplicated; `.`, `..`, a trailing separator, a link or junction all name the real path; on Windows a lower-case drive, forward slashes and `\\?\` name `C:\…` |
| Failure wording | macOS says "not a directory" or "permission denied"; Windows says "already exists" under a file and gives no code for a missing drive, so the runner words those |
| A Cloud | Already creates its directory at every admission; now once |

## Prior art

- Kubernetes' kubelet reports its node's and pods' status to the API
  server, which every client reads; nothing a client reads calls the node
  ([Nodes](https://kubernetes.io/docs/concepts/architecture/nodes/)).
- Coder's workspace agent reports its apps and metadata to the server,
  which the dashboard reads
  ([agent metadata](https://coder.com/docs/admin/templates/extending-templates/agent-metadata)).
- Demi departs in keeping a few reads a user makes of a Host, files and
  live views, which no report can hold.

## Alternatives

- **Cache a Host's answers in memory.** The cache is empty after a restart,
  and an empty answer went uncached, which is how the tabs case arose; a
  stored report has neither gap.
- **A deadline on every inline wait.** The loops would still stall for the
  deadline.
- **Ask whether a directory exists before a move.** A Host asleep or away
  makes the user wait or refuses a move that would work later.

## Open questions

None.
