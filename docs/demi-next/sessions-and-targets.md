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

The backend registers a job before sending it. Its live record binds the device,
root conversation, agent node, shell, and invoking Host. These identities remain
fixed for that job even when another shell starts or another conversation uses
the same device.

For example, node `a1` can run jobs on both the main laptop and attached device
`ci`. The table shows which state a callback accesses:

| Job | Execution device | Callback command state | Invoking Host |
| --- | --- | --- | --- |
| `j1` from `a1` | laptop | `a1`'s tree and storage | laptop |
| `j2` from `a1` | ci | `a1`'s tree and storage | ci |

A callback supplies its job reference. The backend checks the authenticated
device connection and live job record, then validates node/shell identity and
command arguments. Unknown, completed, or disconnected jobs cannot invoke
callbacks. A device token does not authorize another user's jobs.

Cross-host jobs preserve their originating conversation, node, and shell.
Pipe authorization remains bound to the participating device endpoints.
[Commands](commands.md) defines dispatch; the backend runner registry owns
product authorization rather than delegating it to the generic Host adapter.

## Switch the main target

Switching requires an unarchived conversation with an idle agent tree and no
conflicting conversation file operation. A running child counts as active even
when the root is idle and the child is waiting for a provider.

The backend performs one protected transition:

1. Validate the destination and user ownership.
2. Reserve the idle tree. For the planned browser resource, stop new observer
   admission and drain its passive subscriptions as specified below.
3. Reserve conversation file admission and release resources bound to the old
   selection through the reserved Host access described below.
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
That entry does the same work for every kind of target: it resolves the target as the agent
does, refuses an archived conversation, wakes a stopped Cloud and holds it for
the operation, and takes the conversation's file gate so the operation excludes
an archive or a target switch. A paired device without a live runner fails the
operation with the runner's offline error. There is no second way to a host;
an operation that must not wake the Cloud, or must not wait for the gate, is a
change to this rule, decided here, not a bypass in code.

Attached cwd is a starting directory, not a permission boundary. It is updated
from completed cross-host jobs. Files can be transferred explicitly with ordinary
shell pipelines:

```sh
tar c . | demi host shell --host ci 'tar x -C /work'
demi host shell --host ci 'tar c -C /work .' | tar x
```

The backend brokers the byte streams. Attachment changes advance execution context
for each node. Revoking a paired device terminates its connection and removes its
conversation grants.

### Retained resources and passive subscriptions

Planned extension for the [conversation browser](browser.md); not implemented.
A browser frame/registry subscription is a Host operation for its entire admitted
lifetime. It holds normal file admission and device activity, responds to
cancellation, and releases both before acknowledging completion. There is no
permanent lease merely because a browser resource exists.

A target/directory change or archive must not wait forever for a workpanel stream.
After validating the requested transition and reserving the idle tree, the
backend closes new passive-subscription admission, cancels existing observers,
and awaits their release. Other in-flight file operations still obey normal busy
admission; they are not forcibly cancelled by calling them observers.

With observers drained, the transition reserves the conversation file gate.
It then releases resources on the old main target before committing the selection
or archive. This cleanup still calls `withHost`, using an internal reservation
capability issued by that gate. The capability is scoped to this conversation,
expected old selection, and transition; it lets `withHost` use the already-held
exclusive reservation rather than acquiring a conflicting shared gate. It does
not skip ownership, binding, archive checks, Cloud wake/hold, or IO cancellation.
It cannot be supplied through a request body, shell argument, or environment.
The transition has not committed yet, so the old binding is still authoritative.

A resource already reported lost has no live Host state to release: revoke its
local grant and record loss without a new Host call. An unconfirmed cleanup
failure is not success; fail the transition and report it unless connection or
service loss has established that the resource generation ended. Cleanup errors
must not be ignored to force the target commit. If release succeeds but the
subsequent database commit fails, the old selection remains and its next browser
open starts fresh; page state cannot be rolled back.

All exits release tree/file reservations and reopen observer admission against
the actual final selection, unless it is archived. Reconnection requires a new
subscription and current generation. Never reuse an old Host object outside
`withHost`, including for cleanup. Detachment and revocation retain their existing
rules; revocation's connection loss invalidates associated resource grants.

## Coordinate shared Cloud activity

Conversation and device admission protect different resources:

| Scope | Protected transition | Work that prevents an idle transition |
| --- | --- | --- |
| One conversation tree | Target change | Root and child turns, restores, queued work, and wakeups admitted by the tree lifecycle |
| Conversation files | Target change | Host operations: uploads, the working tree, file text |
| One Cloud device | Shutdown or reset | Device operations and relevant agent trees across all of its user's conversations |

The managed-host lifecycle reserves device admission before changing the machine.
Tree reservations alone cannot protect a Cloud device shared by multiple
conversations. Normal wake can be joined. Reset rejects new work with a resetting
status, interrupts device work, and coordinates the durable disk transition in
[managed hosts](managed-hosts.md). It does not silently replay interrupted work.

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
routing; `managed/` owns device admission. The Host adapter carries live execution
facts without knowing user policy. [Package boundaries](../package-boundaries.md)
defines their dependencies.

Backend scenarios and agent tests must cover child activity during switches,
concurrent admission, file uploads, cross-user refusal, expired jobs, cross-host
command storage, per-node context updates, and shared-device reset and recovery.
Use scripted providers for these checks.

The current runtime has one owning backend process. Multi-worker failover requires
fencing the old worker's storage and execution authority, in addition to routing
requests to a user owner. That is a scaled-deployment requirement, not an existing
single-instance guarantee.
