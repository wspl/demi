# Conversation execution targets and coordination

A conversation owns one backend agent tree and one execution-target selection.
Each node keeps its own transcript and command storage. The target determines
where an action runs; changing it does not relocate conversation state or files.
This document owns target selection, job identity, switching, and admission.

## Resolve a target

The root and subagents resolve the conversation's selection for each action.
The backend supplies a RemoteHost and a remote shell environment for the selected
device.

| Selection | Device | Initial directory |
| --- | --- | --- |
| Cloud, the default | The user's unique managed device | `/home/demi/sessions/<conversationId>` |
| Paired device | The selected device | The selected directory or reported home |
| Workspace | The workspace's device | The workspace path |

A workspace is a named `(user, device, path)` record. It groups conversations but
does not own a VM or restrict filesystem access. Default Cloud project paths are
`/home/demi/projects/<workspaceId>`. A user can also select an existing directory.
All of a user's Cloud directories share one machine; different users have separate
managed devices and writable disks.

Resolving target metadata does not allocate or start Cloud. The first file,
shell, or process-backed provider operation acquires the device. Concurrent first
uses join one allocation and boot; the control database enforces device uniqueness.
A failed wake reports an error instead of selecting another target.
Cloud sleep is not unavailability: selecting or using a Cloud target must not
require a live runner in advance. The operation joins wake and waits for access.

Scripts run in embedded brush on the target. Cloud jobs use the guest account;
paired-device jobs use the device account. Login profiles, cwd between jobs,
background work, and cancellation follow the [runner job contract](runner.md#shell-jobs).
[Managed hosts](managed-hosts.md) defines Cloud permissions and persistent volumes.

## Bind jobs to their caller

The backend registers a job before sending it. Its live record binds the
device, the shell and the invoking Host, and holds the job's
[command context](native-runtime.md#command-context): the root conversation and
the agent node. These identities remain fixed for that job even when another
shell starts or another conversation uses the same device.

For example, node `a1` can run jobs on both the main laptop and attached device
`ci`. The table shows which state a callback accesses:

| Job | Execution device | Callback command state | Invoking Host |
| --- | --- | --- | --- |
| `j1` from `a1` | laptop | `a1`'s tree and storage | laptop |
| `j2` from `a1` | ci | `a1`'s tree and storage | ci |

A callback supplies only its job reference. The backend checks the
authenticated device connection and the live job record, gives the handler the
command context from that record, and validates the command arguments. Unknown, completed, or disconnected jobs cannot invoke
callbacks. A device token does not authorize another user's jobs.

A cross-host job carries its originating job's command context.
Pipe authorization remains bound to the participating device endpoints.
[Commands](commands.md) defines dispatch; the backend runner registry owns
product authorization rather than delegating it to the generic Host adapter.

## Switch the main target

Switching requires an unarchived conversation with an idle agent tree and no
conflicting conversation file operation. A running child counts as active even
when the root is idle and the child is waiting for a provider.

The backend performs one protected transition:

1. Validate the destination and user ownership.
2. Reserve the idle tree.
3. Reserve conversation file admission and send the
   [conversation release](resource-lifecycle.md#conversation-release) to the old
   device when it is connected.
4. Commit the target and attachment changes against the expected old selection.
5. Advance the execution-context revision and release all reservations. Reopen
   observer admission against the resulting selection.

Concurrent changes cannot both replace the same expected selection. The backend
reports busy or conflict rather than dispatching work against an ambiguous target.
Turn admission uses the same tree lifecycle boundary as the reservation; checking
only the root's displayed phase is insufficient.

For a switch from an allocated Cloud device to a laptop, the result is:

| State | Before | After |
| --- | --- | --- |
| Main device | Cloud | laptop |
| Attached devices | None | Cloud, with its last cwd |
| `report.txt` created on Cloud | On Cloud | Still on Cloud |
| Conversation transcript | Backend | Same backend transcript |

The incoming device leaves the attachment set. Switching directories on the same
device does not add a duplicate attachment. Previous paths continue to refer to
their original device; another directory on the same Cloud machine remains
locally accessible.

Each node observes the latest execution-context revision before its next inference.
Its persisted context block describes the switch, attached hosts, and any Cloud
reset. Observation is node-specific: the root seeing an update does not consume
it for a child. Product execution context is independent of custom profile prompts.

## Attached hosts

The user grants access by attaching devices; the agent cannot attach a device
for itself. Each device appears once across the main and attached bindings.
Aliases begin with device names and use numeric suffixes for collisions. The
host API supports alias changes, promotion through target exchange, and detach.

`demi host list` reports accessible main and attached hosts.
`demi host shell --host <name|id> <script>` verifies ownership and the conversation
binding before starting a job. A sleeping Cloud device wakes for work; attachment
alone does not keep it running. The product picker offers paired devices, while
Cloud can become attached when it is a departed main target.

## Host operations

Everything that touches a conversation's main or attached host on the conversation's
behalf from outside the agent, whether it writes an attachment, lists the
working tree, or reads a file for the browser, goes through one entry, the
conversation's host access (`ConversationTargets.withHost`). An operation can name
a device bound as the main or an attached host; omitting the device selects the
current main host. The binding and ownership are checked after taking the file
gate and before reaching the Host. An unknown or detached device is refused;
detaching prevents new access, while an already admitted operation may finish.
For normal demand, that entry does the same work for every kind of target: it
resolves the target as the agent does, refuses an archived conversation, wakes
a stopped Cloud and holds it for the operation, and takes the conversation's file gate so the operation excludes
an archive or a target switch. A paired device without a live runner fails the
operation with the runner's offline error. There is no second way to a host.
Lifecycle cleanup is the explicitly scoped extension below; it still uses this
entry. Any other change to wake or gate behavior must be decided here, not
introduced as a bypass in code.

Two operations last as long as the browser decides. The first is a file
transfer: the bytes of a [file preview](file-previews.md) or a download, which
last as long as a user watches a video, and of an upload, which last as long
as the browser takes to send them. It holds Host access like any operation, until its
last byte is delivered or the browser ends it. Two rules keep a forgotten
transfer from holding a Cloud awake or a conversation's file gate:

- A transfer the browser has accepted no bytes from, or sent none to, for 60
  seconds releases its access and stops the runner's read or write. Only time
  spent waiting for the browser counts; waiting for the runner or for a Cloud
  to wake does not. A download's response is left without an end, so when its
  connection closes the browser sees it cut short, never complete, and a
  paused player asks again for the range it still needs; an upload's file is
  left as it was. A connection on which no byte has moved for 60 seconds is
  closed. Sixty seconds is the stalled-client timeout web servers use,
  nginx's `send_timeout` among them.
- An archive, a target or directory change, and a detach end the
  conversation's open transfers instead of waiting for them or being refused
  by them. A Cloud stop or reset ends them with the device's other work.

The second is a [user stream](native-runtime.md#user-streams), such as the
[live browser view](browser-live-view.md), open for as long as the page shows
it. Its admission checks ownership and the binding and refuses an archived
conversation, like any operation, with two differences:

- It never wakes a stopped Cloud: a stopped Cloud holds none of the state a
  user stream shows, so the page learns that the Host is stopped. Work the
  user starts from the view, such as opening a new browser tab, is ordinary
  demand and wakes it.
- Once admitted, it does not hold the conversation's file gate or keep a Cloud
  awake. Its traffic is retention, not activity; the user operations it carries
  report activity separately ([Activity](resource-lifecycle.md#activity)).

An archive, a target or directory change, and a detach end the conversation's
user streams as they end file transfers; a Cloud stop or reset ends them with
the device's other work.

One kind of access reaches a device without a conversation: the public relay
of a [Host expose](expose.md#the-public-relay), whose traffic comes from
anonymous visitors and belongs to the user's device, not to any
conversation. It uses device access: the registry's Host for a device the
expose's owner owns, admitted only while the device is connected. It takes
no file gate, because it touches no conversation files, and never wakes a
stopped Cloud, because a stop has already destroyed the device's exposes.
Device access exists for that one caller; anything on a conversation's
behalf still goes through `withHost`.

Attached cwd is a starting directory, not a permission boundary. It is updated
from completed cross-host jobs. Files can be transferred explicitly with ordinary
shell pipelines:

```sh
tar c . | demi host shell --host ci 'tar x -C /work'
demi host shell --host ci 'tar c -C /work .' | tar x
```

The backend brokers the byte streams. Attachment changes advance execution context
for each node. Revoking a paired device terminates its connection, which ends every
conversation's state on it.

### Lifecycle access

Cleanup admission for [conversation idle and release](resource-lifecycle.md).

Ordinary access acquires conversation and device admission before Host IO. If a
resource transition wins between target resolution and admission, release the
partial gates before waiting outside `withHost`'s acquisition attempt. Recheck
authorization, archive status, and current binding on re-entry. No caller may
hold a conversation file gate while waiting for a device transition that needs
that gate for dependent cleanup. Do not retry after operation dispatch; only the
unstarted admission attempt may wait and re-enter. Reset/archive/loss preserve
their explicit refusal semantics.

A target/directory change or archive validates its request, reserves the idle
tree, cancels existing viewers, file transfers and user streams, and awaits
their release.
Other file work follows normal busy admission and is not silently classified
as a passive observer. After reserving conversation file admission, it sends the conversation
release to the old device through the conversation's host access, then commits
the target/archive change. The old binding remains authoritative until commit.
The release is a runner message, not Host IO: it needs no file gate, never
wakes a stopped Cloud, and is skipped when the device is not connected, because
connection loss has already ended the conversation's state there.

If the database commit fails after the release, the old selection remains and
its next browser open starts fresh; page state cannot be rolled back. Device
revocation and unexpected loss end the conversation's state on that device
without using Host access to revive it for cleanup.

## Coordinate shared Cloud activity

Conversation and device admission protect different resources:

| Scope | Protected transition | Work that prevents an idle transition |
| --- | --- | --- |
| One conversation tree | Target change | Root and child turns, restores, queued work, and wakeups admitted by the tree lifecycle |
| Conversation files | Target change | Host operations: uploads, the working tree, file text. File transfers and user streams are ended, not awaited. |
| One Cloud device | Shutdown or reset | Device operations, and the agent trees of the conversations whose target is that Cloud |

The lifecycle module reserves device admission before the managed-host
adapter changes the machine.
Tree reservations alone cannot protect a Cloud device shared by multiple
conversations. Normal wake can be joined. Reset interrupts device work and
coordinates the durable disk transition in [managed hosts](managed-hosts.md).
It does not silently replay interrupted work.

A reset reaches a conversation only through its target. It interrupts and
holds the conversations whose target is that Cloud, and no others. A
conversation that merely has the Cloud attached keeps running on its own
target: its turn is not interrupted, its streams stay open, and it can be
opened, read and written throughout. A command it had running on the Cloud
ends with the device, as it would on any Host that went away, and its next
Cloud command waits for the device like any other wake.

A held conversation is waiting, not failed. While a transition holds a
conversation (a reset, a target change, an archive), everything the user sends
to it waits for the transition and then proceeds: opening it, a message, a
steer, a Host operation. Nothing is answered with a refusal because the
transition is running, so nothing is left for the user to retry, and the
conversation is usable the moment the transition ends. A wait ends early only
when its requester goes away. User streams and file transfers are the
exception the table names: a transition ends them instead of waiting for them,
and the page reopens its streams by itself afterwards. The page shows that the conversation is waiting
and why (`Cloud is resetting`), from the Cloud status it already follows.

For example, an idle conversation cannot cause Cloud shutdown while another
conversation is using that same device. A callback remains bound to the job that
originated it throughout the shared-device activity.

## Recovery and persistence

An unavailable paired device or failed Cloud wake produces an operation error.
History remains readable, and the backend does not redirect the operation.
Browser disconnect does not abort the agent turn; a reconnecting browser
synchronizes with the backend transcript.

The agent persists a tool call as executing before dispatch. Recovery distinguishes
these cases:

| Stored state | Recovery |
| --- | --- |
| Result recorded | Preserve the result. |
| Dispatched, result absent | Record an unknown outcome; do not automatically replay side effects. |
| Not dispatched | Continue through the normal resume path. |

Runner connection loss and subsequent reconnection follow the
[runner lifetime contract](runner.md#command-lifetime). New connections do not
resurrect old jobs.

Conversation trees and command state remain in backend storage. Working files
remain on their devices. Cloud system and home volumes persist across ordinary
shutdown; reset preserves home. Archiving a conversation does not delete its
session directory or Cloud machine. Workspace deletion requires no referencing
conversations and deletes metadata, not its directory.

Full shell output stays on the execution device. Cloud runner output under
`/run/demi` is temporary and can disappear on shutdown or reset. Durable results
must be written to persistent directories; the backend transcript retains only
the recorded output view.

## Implementation ownership and checks

The agent owns tree admission and per-node context persistence. Backend
`conversation/` owns target transitions; `runner/` owns authenticated callback
routing; `lifecycle/` owns the conversation idle clock and the conversation
release, while `managed/` owns Cloud policy and machine transitions; see
[Conversation idle and Host resource release](resource-lifecycle.md). The Host
adapter carries live execution facts without knowing user policy. [Package boundaries](../package-boundaries.md)
defines their dependencies.

Backend scenarios and agent tests must cover child activity during switches,
concurrent admission, file uploads, cross-user refusal, expired jobs, cross-host
command storage, per-node context updates, and shared-device reset and recovery.
Use scripted providers for these checks.

The current runtime has one owning backend process. Multi-worker failover requires
fencing the old worker's storage and execution authority, in addition to routing
requests to a user owner. That is a scaled-deployment requirement, not an existing
single-instance guarantee.
