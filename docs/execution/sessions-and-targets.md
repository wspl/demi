# Conversation execution targets and coordination

A conversation owns one backend agent tree and one execution-target selection.
Each node keeps its own transcript. The target determines
where an action runs; changing it does not relocate conversation state or files.
This document owns target selection, job identity, switching, admission, and
every way the backend reaches a Host.

## Resolve a target

The root and subagents resolve the conversation's selection for each action.
The backend gives them the selected device's Host, reached through that
device's runner connection, and a shell environment on that Host.

| Selection | Device | Initial directory |
| --- | --- | --- |
| Cloud, the default | The user's unique managed device | `/home/demi/sessions/<conversationId>` |
| Paired device | The selected device | The selected directory or reported home |
| Workspace | The workspace's device | The workspace path |

A reported home is the one the device's runner sent when it last connected.
The backend keeps it in memory only, so after a backend restart the home is
unknown until the runner connects again; no operation reaches the device before
then.

A workspace is a named `(user, device, path)` record. It groups conversations but
does not own a sandbox or restrict filesystem access. Revoking its device
removes the workspace but not its files; its conversations stay outside any
project and cannot run there any more ([Web API](../product/web-api.md)). Default Cloud project paths are
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
paired-device jobs use the device account. Login profiles, the directory each
job starts in, background work, and cancellation follow the [runner job contract](runner.md#shell-jobs).
[Managed hosts](../cloud/managed-hosts.md) defines Cloud permissions and persistent volumes.

## Bind jobs to their caller

The backend registers a job before sending it. Its live record binds the
device, the shell and the invoking Host, and holds the job's
[command context](native-runtime.md#command-context): the root conversation and
the agent node. These identities remain fixed for that job even when another
shell starts or another conversation uses the same device.

For example, node `a1` can run jobs on both the primary laptop and attached device
`ci`. The table shows which node a callback acts for:

| Job | Execution device | Callback node | Invoking Host |
| --- | --- | --- | --- |
| `j1` from `a1` | laptop | `a1` | laptop |
| `j2` from `a1` | ci | `a1` | ci |

A callback supplies only its job reference. The backend checks the
authenticated device connection and the live job record, gives the handler the
command context from that record, and validates the command arguments. Unknown,
completed, or disconnected jobs cannot invoke callbacks. A device token does not
authorize another user's jobs.

A cross-host job carries its originating job's command context.
Pipe authorization remains bound to the participating device endpoints.
[Commands](commands.md) defines dispatch. The backend authorizes a callback
where it holds the device's runner connection, in the owner's shard, rather
than delegating product authorization to the generic Host adapter.

## Switch the primary target

Switching requires an unarchived conversation with an idle agent tree and no
conflicting conversation file operation. A running child counts as active even
when the root is idle and the child is waiting for a provider.

The backend performs one protected transition:

1. Validate the destination and user ownership.
2. Reserve the idle tree.
3. End the conversation's file transfers, user streams and the one-shot user
   calls admitted like them, and wait for their release.
4. Reserve conversation file admission. When the switch leaves the old device
   and that device's runner is connected, a paired device's or a running
   Cloud's, send it the
   [conversation release](resource-lifecycle.md#conversation-release).
5. Commit the target and attachment changes against the expected old selection.
6. Advance the execution-context revision and release all reservations. Admit
   file transfers and user streams again, against the resulting selection.

Concurrent changes cannot both replace the same expected selection. The backend
reports busy or conflict rather than dispatching work against an ambiguous target.
Turn admission uses the same tree lifecycle boundary as the reservation; checking
only the root's displayed phase is insufficient.

For a switch from an allocated Cloud device to a laptop, the result is:

| State | Before | After |
| --- | --- | --- |
| Primary device | Cloud | laptop |
| Attached devices | None | Cloud, its commands starting in the conversation's directory there |
| `report.txt` created on Cloud | On Cloud | Still on Cloud |
| Conversation transcript | Backend | Same backend transcript |

The incoming device leaves the attachment set. Switching directories on the same
device does not add a duplicate attachment. Previous paths continue to refer to
their original device; another directory on the same Cloud machine remains
locally accessible.

**Telling the agent.** A switch the user makes may tell the agent at once
([Where a conversation runs](../product/product.md#where-a-conversation-runs)):
with `notifyAgent`, the backend commits the switch and then admits a message
from the user into the root, through the entry a permission decision uses
([The decision's message](../agent/permissions.md#the-decisions-message)), so
it wakes the idle root with a turn. Like a decision, it is an agent message
whose sender is the user, so it continues the work so far and starts no
request ([Edit tracking](edit-tracking.md#a-request)). Its event is `moved`,
and the model reads:

```text
The user moved this conversation from Cloud (/home/demi/sessions/0f4e…) to
MacBook Pro (/Users/zan/code/ledable-app). Files did not move. Check what this
means for the work so far, and tell the user.
```

followed, as before any inference, by the new execution-context block. The
message's id is `moved:<revision>`, the execution-context revision the switch
made, so delivering it again changes nothing. Without `notifyAgent` nothing
is admitted, and the agent reads the switch from the block at its next turn.

**A switch the agent asks for.** An agent's command that moves its own
conversation ([What the agent can change](#what-the-agent-can-change)) runs
while its tree works, which a switch refuses. The command therefore records
the switch as the conversation's **pending move** and answers at once; the
backend makes it, as above, when the tree is next idle, before it admits
anything else, so the next turn, the user's or a report's, runs in the new
place and reads it from its block. A newer pending move replaces an older
one; an archive drops it, and an agent's pending detaches with it. A pending move is stored with the conversation
([Storage](../backend/storage.md#control-records)), so a restart does not
lose it. Making it checks what any switch checks, not whether the device is
online, as the user's own switch does not: a move to an offline device is
made, and the next turn finds the device offline as any conversation there
does. A pending move that fails when it is made, such as into a project
deleted meanwhile, leaves the conversation where it was and wakes the root
with an agent message through the same entry, event `move_failed`, id
`move-failed:<move id>`, whose envelope names Demi as the sender, a notice of
the product's and not agent-originated context. The model reads:

```text
Demi could not make the move this conversation's agent asked for: the
project no longer exists. The conversation still runs on Cloud
(/home/demi/sessions/0f4e…).
```

The notice names no project: the agent's own transcript holds the command it
ran, and a deleted project's name is gone with it. The transcript shows the
notice as a receipt row, *Move failed*, which expands to the message, for a
move into a project and out of one alike. A move that succeeds sends nothing: the agent
asked for it, and its next block names the new place.

Each node observes the latest execution-context revision before its next inference,
the first one included: a node's first inference is preceded by a block of
the current revision, so the agent never has to probe where it runs. For
example, a new conversation on the user's Cloud starts with:

```text
[Execution context 0]
Primary host: Cloud, Ubuntu 26.04 (x86_64). Shells start in /home/demi/sessions/0f4e7597-e889-4baa-ac98-93fa61b68d28. Standard utilities (sed, grep, cp, ls, find, …) are GNU's on every host.
Attached hosts: none. `demi host list` shows every host this conversation can reach.
```

Every block names the primary Host this way: its name, its operating system
and release as its runner last reported them with the architecture, the
directory its shells start in, and that the standard utilities are GNU's
([Standard utilities](runner.md#standard-utilities)). On macOS it adds where
the system's own are: `Standard utilities (sed, grep, cp, ls, find, …) are
GNU's on every host; macOS's own are in /usr/bin.` A Host whose runner never connected, such as
a Cloud that has not run yet, is named without the system.
Its persisted context block describes the switch, attached hosts, and any Cloud
reset. Observation is node-specific: the root seeing an update does not consume
it for a child. Product execution context is independent of custom profile prompts.
The node's own context blocks are the record of what it saw. A block names its
revision, and a node whose transcript holds the current one gets no block. A
switch is announced to a node once: the previous and current targets, that no
files moved, and how to reach the device left behind with `demi host shell
--host`. A later change is announced as a change of the attached hosts. Every
block ends with the attached hosts, each with its connection and the directory
its shells start in.

## Attached hosts

The conversation's agents attach the devices their work needs, with
`demi host attach`, which the user allows once per conversation
([What the agent can change](#what-the-agent-can-change)); the product offers
no way to attach one by hand, and the user detaches one from the header's
menu ([Where a conversation runs](../product/product.md#where-a-conversation-runs)).
Each device appears once across the primary and attached bindings.
Aliases begin with device names, or the name `demi host attach --name`
gives, and use numeric suffixes for collisions. Choosing an attached device
as the primary Host exchanges the two.

`demi host list` reports accessible primary and attached hosts.
`demi host shell --host <name|id> <script>` verifies ownership and the conversation
binding before starting a job. A sleeping Cloud device wakes for work; attachment
alone does not keep it running. `demi host attach` attaches a paired
device; the Cloud becomes attached when it is a departed primary target. A
new conversation's draft has no attached hosts, and the target it leaves
when the user moves it is not attached, since nothing has run there.

Every command on an attached host starts in that host's starting directory:
the conversation's directory there for a departed primary target, and the
device's home for a device `demi host attach` attached. It is fixed when the
host is attached, never moved by a command's `cd`, as on the primary Host
([Running shell tools](../agent/runtime.md#running-shell-tools)), and it is not
a permission boundary. Files can be transferred explicitly with ordinary
shell pipelines:

```sh
tar c . | demi host shell --host ci 'tar x -C /work'
demi host shell --host ci 'tar c -C /work .' | tar x
```

The backend brokers the byte streams. Attachment changes advance execution context
for each node. Revoking a paired device terminates its connection, which ends every
conversation's state on it.

## What the agent can change

The agent can change where its own conversation runs, its title and the
devices it reaches, with commands of two [permission
categories](../agent/permissions.md#categories), each allowed once per
conversation:

| Command | Does | Category |
| --- | --- | --- |
| `demi conversation rename "<title>"` | Renames this conversation ([Conversation titles](../product/product.md#conversation-titles)) | `conversation.organize` |
| `demi conversation projects` | Lists the user's projects: name, Host, directory | `conversation.organize` |
| `demi conversation move <project>` | Moves this conversation into a project, by name or id | `conversation.organize` |
| `demi conversation move --out` | Moves it out of its project, to the same directory on the same Host | `conversation.organize` |
| `demi conversation create-project <name> [<directory>]` | Makes a project of a directory on the primary Host, its shell's directory by default, and moves this conversation into it | `conversation.organize` |
| `demi host devices` | Lists the user's paired devices and the Cloud, with their states | `host.devices` |
| `demi host attach <device> [--name <alias>]` | Attaches a device to this conversation | `host.devices` |
| `demi host detach <host>` | Detaches one | `host.devices` |

The categories:

| Id | Action | Description |
| --- | --- | --- |
| `conversation.organize` | organize conversations | Organize Conversations lets the agents of this conversation rename it, list your projects, make a project of a directory, and move this conversation into or out of one. Projects show in your sidebar on every device. |
| `host.devices` | manage devices | Manage Devices lets the agents of this conversation list your devices, and attach them to this conversation or detach them. The agents run commands as you on an attached device. |

`move` and `create-project` change where the conversation runs, and make a
[pending move](#switch-the-primary-target) that applies when the tree is next
idle; they print that the move applies when this conversation's work ends.
`detach` is a transition too, which the command's own job and its working
tree would always find busy, so it waits the same way: it marks the
attachment to end, prints that the device is detached when this
conversation's work ends, and the backend detaches it then, in the same
transition as a pending move; the device stays usable until then, and an
`attach` of it before then cancels the mark. `rename` and `attach` apply at
once. An attach or detach is announced to every node in its next
execution-context block, as one the user makes. A move into a project on a paired device that is neither the primary
Host nor attached brings that device into the conversation, so it also needs Manage
Devices, and one request asks for both
([Several categories](../agent/permissions.md#several-categories)).

## Host operations

Everything that reaches a conversation's primary or attached Host on the
conversation's behalf goes through one entry, the conversation's host access.
The agent resolves its Host through it for each tool call and runs each shell
job inside it, and everything the backend does for the user outside the agent
uses it too, whether it writes an attachment, lists the working tree, or reads
a file for the web app. A plugin never writes to a Host: the
[Host directories](../architecture/plugins.md#host-directories) its user
needs are installed by a job's own admission, before the job starts, and its
reads of a conversation's files never wake the Host. An operation can name a device bound as the primary or an
attached host; omitting the device selects the current primary host. The binding
and ownership are checked after taking the file gate and before reaching the
Host. An unknown or detached device is refused; detaching prevents new access,
while an already admitted operation may finish. A target whose device no
longer exists, such as a revoked device, is refused the same way. For normal demand, that entry does the same work
for every kind of target: it resolves the target, refuses an archived
conversation, wakes a stopped Cloud and holds it for the operation, and takes
the conversation's file gate so the operation excludes an archive or a target
switch. A paired device without a live runner fails the operation with the
runner's offline error. A `shell` call gets it as its result, worded for the
model so it stops trying: `browse-slot-1 is offline: its runner has been
disconnected for 40 seconds, so nothing can run there now. Commands already
running there are kept for up to 10 minutes and report when it is back; end
your turn to wait for them, or tell the user the device is offline.` In a
product check, a model told only `runner disconnected` ran seven more
commands to ping the device. There is no second way to a conversation's Host; the
two ways to a device that touch no conversation's files are named in
[Every way to a Host](#every-way-to-a-host). Lifecycle cleanup is the explicitly
scoped extension in [Lifecycle access](#lifecycle-access); it still uses this
entry. Any other change to wake or gate behavior must be decided here, not
introduced as a bypass in code.

The conversation's host access runs in the user's shard, the part of the
backend that holds everything belonging to one user on a single thread
([The user shard](../architecture/concurrency.md#the-user-shard)). The file
gates, the admission of file transfers and user streams, and the transitions
that end them live there too. A runner connection makes a conversation's Host
handle only against a lease of that conversation's file gate, which only the
host access takes, so no other code can make one
([`backend-runners`](../architecture/crates-and-packages.md#backend-runners)).

Admission is a loop, because the two things it waits for, the conversation's
file gate and a Cloud's admission, are also what a transition takes. A Cloud's
admission is the hold that keeps a Cloud running: taking it wakes a stopped
Cloud, joins a boot already under way, or waits for a running reset to finish.

```text
1. Take the conversation's file gate.         Waits while a transition holds it.
2. Resolve the target; check the archive,     Refused: an archived conversation,
   the binding and the owner.                 a device that is not bound.
3. If the Host is a Cloud whose admission
   the attempt does not hold yet: release     Wakes a stopped Cloud;
   the file gate, take the Cloud's            waits for a running reset.
   admission, and go back to 1.
4. Run the operation, once.
5. Release the file gate and the Cloud's admission.
```

The file gate is released before waiting for a Cloud because a reset takes the
file gates of the conversations it holds: a caller that kept its file gate
while waiting for that reset would wait forever. For the same reason nothing
enters a conversation's file gate while it holds a lease of it: the gate is
first-in, first-out, so a reservation waiting between the two entries would
wait for the first lease while the second waits behind the reservation. A
conversation socket's frame is handled under one lease of the file gate; a
frame whose content has uploads is instead admitted on the conversation's primary
Host once, through this entry, and its uploads are written through that
admission. Going back to the start
repeats every check, because while the attempt waited the conversation may
have been archived or switched to another target, or its device detached. Only
an attempt that has not dispatched its operation waits and starts over. An
operation already dispatched runs once and is never retried: if a reset or the
loss of its device cuts it off, it fails with that cause.

Two operations last as long as the user's browser decides. The user's shard
admits each one like any operation, and the backend's edge carries its bytes:
the part of the backend that serves HTTP and moves bytes on threads shared
by all users ([Programs and threads](../architecture/concurrency.md#programs-and-threads)).
With the pipe ends, the shard hands the edge a lease. The edge holds the lease
while bytes move and drops it when it is done, which releases the admission in
the shard. When the shard ends the operation first, it releases the admission
at once, without waiting for the edge, and the edge sees the lease end and
stops. For example, a download of `report.txt` from the Cloud:

```text
Web app            Edge                          User's shard                 Runner
  |-- GET fs/raw -->|                               |                            |
  |                 |-- admit the download -------->| take the file gate and     |
  |                 |                               | the Cloud's admission      |
  |                 |                               |-- read into a pipe ------->|
  |                 |<-- pipe end and lease --------|                            |
  |<-- bytes -------|<-- bytes from the pipe ------------------------------------|
  |                 |-- last byte sent: drop lease->| release both               |
```

The first is a file transfer: the bytes of a
[file preview](../product/file-previews.md) or a download, which last as long as
a user watches a video, and of an upload, which last as long as the user's
browser takes to send them. It holds Host access like any operation, until its
last byte is delivered or the user's browser ends it. Two rules keep a forgotten
transfer from holding a Cloud awake or a conversation's file gate:

- A transfer the user's browser has accepted no bytes from, or sent none to, for
  60 seconds releases its access and stops the runner's read or write. The edge
  applies this rule, since only the edge sees the user's browser's pace. Only
  time spent waiting for the user's browser counts; waiting for the runner or
  for a Cloud to wake does not. A download's response is left without an end, so
  when its connection closes the user's browser sees it cut short, never
  complete, and a paused player asks again for the range it still needs; an
  upload's file is left as it was. A connection on which no byte has moved for
  60 seconds is closed. Sixty seconds is the stalled-client timeout web servers
  use, nginx's `send_timeout` among them.
- An archive, a target or directory change, and a detach end the
  conversation's open transfers instead of waiting for them or being refused
  by them. A Cloud stop or reset ends them with the device's other work.

The second is a [user stream](native-runtime.md#user-streams), such as the
[live browser view](../browser/live-view.md), open for as long as the page
shows it. The shard hands the edge the stream's pipe ends and a lease, as it
does for a transfer, and the edge relays the stream's bytes between the page
and the Host. Its admission checks ownership and the binding and refuses an
archived conversation, like any operation, with two differences:

- It never wakes a stopped Cloud: a stopped Cloud holds none of the state a
  user stream shows, so the page learns that the Host is stopped. Work the
  user starts from the view, such as opening a new conversation browser tab,
  is ordinary demand and wakes it.
- Once admitted, it does not hold the conversation's file gate, so a
  transition ends it instead of waiting for it. While it is open the
  conversation is active, which keeps a Cloud awake
  ([Activity](resource-lifecycle.md#activity)).

An archive, a target or directory change, and a detach end the conversation's
user streams as they end file transfers; a Cloud stop or reset ends them with
the device's other work. A one-shot user call that must not wake the Host,
such as listing or closing the conversation browser's tabs, is admitted the
same way and ends the same way: a transition ends it instead of waiting for it
to finish. A call that operates the Host, such as closing a tab, is activity; a
look at what runs there, such as listing the tabs, is not.

The [file watch](../product/web-api.md#file-watch), which carries the Host's
reports of changed files to a page, is admitted and ended as a user stream is,
and never wakes a stopped Cloud, whose files cannot change. It is not
activity: a page that shows files does not keep a Cloud awake. When a stopped
Cloud wakes, the page opens its watch again.

Backend shutdown ends every open transfer and user stream before it saves and
stops the user's Cloud, so a download left open never keeps a Cloud from being
saved.

### Every way to a Host

The conversation's host access is the only way to a conversation's Host, in the
forms the first rows name, but for a direct channel, which device access
introduces. Two more ways reach a device for work that
touches no conversation's files. This table names every way to a Host:

| Way | Used by | Takes | A stopped Cloud |
| --- | --- | --- | --- |
| The conversation's host access | The agent's tool calls and shell jobs, with the plugins' Host directories each job needs; attachments, the working tree, file text and file transfers; one-shot user calls that start work, such as opening a conversation browser tab | The conversation's file gate while the operation runs, and, for a Cloud, the Cloud's admission | Woken; a running reset is waited for |
| User-stream admission, a form of the conversation's host access | User streams; one-shot user calls that must not wake the Host; a plugin's [reads of the conversation's files](../architecture/plugins.md#reading-a-conversations-files) | The file gate while admitting only | Not woken: the caller learns that the Host is stopped |
| [Lifecycle access](#lifecycle-access), a form of the conversation's host access | The conversation release | Nothing: the release is a runner message, sent only to a connected runner | Not woken: the Cloud hears the release after its next wake |
| Device access | A [direct channel](direct-channel.md) between the page and a paired device's runner, which carries the page's user streams, file transfers, file text, listings and file watch of the user's conversations on that device, and the relay probes that measure its paths; the [device log](../product/web-api.md#device-log); browsing a paired device's directories to choose a target; the [Claude Code versions](../providers/claude-code.md#what-the-user-sees) a provider's settings show | Nothing: the caller must own the device, and its runner must be connected | Not woken |
| Machine access | Creating a Cloud project; [placing a provider's process](../providers/claude-code.md#where-it-runs) on the user's Cloud; [a Claude Code sign-in](../providers/claude-code.md#accounts-and-sign-in), which runs the CLI's login there | The Cloud's admission; no file gate | Woken; a running reset is waited for |

Device access and machine access touch no conversation's files, so they take no
file gate and check no conversation's binding. Device access reaches a device
the caller owns only while its runner is
connected. It never wakes a stopped Cloud: a log is read when its Host runs again, and directory
browsing never names a Cloud, and showing a version is never a reason to wake
one. Machine access is for work that needs the user's
Cloud itself rather than a conversation's files on it. Creating a Cloud project
makes the project's directory before any conversation uses it. A provider's
process runs in a directory of Demi's on the Cloud, for a conversation's
requests (the [`provider` role](#how-a-conversation-uses-a-device)) and for
account work that belongs to no conversation, such as **Test connection**.
Anything else that reaches a conversation's primary or attached Host goes
through the conversation's host access. A direct channel is the device
owner's work on their own device, as browsing its folders is: the backend
checks ownership when it introduces the page, and adds no conversation gate,
so a transition does not wait for the channel's operations, and the page
stops sending a conversation's operations to a device once its summary names
another Host.

### Lifecycle access

Cleanup admission for [conversation idle and release](resource-lifecycle.md).

A target/directory change or archive validates its request, reserves the idle
tree, ends the conversation's file transfers, user streams and the one-shot
user calls admitted like them, and awaits their release. Other file work
follows normal busy admission and is not silently classified as a passive
observer. After reserving conversation file admission, it sends the
[conversation release](resource-lifecycle.md#conversation-release) through the
conversation's host access to each device the change leaves, then commits the
target/archive change, also when a release failed
([A release that fails](resource-lifecycle.md#a-release-that-fails)). The old
binding remains authoritative until commit.
The release is a runner message, not Host IO: it needs no file gate, and it is
skipped for a device whose runner is not connected, a stopped Cloud among
them, because the connection loss or the stop has already ended the
conversation's services there, and the device holds no files of the
conversation for a release to remove
([Conversation release](resource-lifecycle.md#conversation-release)).

If the database commit fails after the release, the old selection remains and
the conversation browser starts fresh at its next open; the state of its tabs
cannot be rolled back. Device revocation and unexpected loss end the conversation's state
on that device without using Host access to revive it for cleanup.

## Coordinate shared Cloud activity

Conversation and device admission protect different resources:

| Scope | Protected transition | Work that prevents an idle transition |
| --- | --- | --- |
| One conversation tree | Target change | Root and child turns, restores, queued work, and command reports admitted by the tree lifecycle |
| Conversation files | Target change | Host operations: uploads, the working tree, file text. File transfers, user streams and the one-shot user calls admitted like them are ended, not awaited. |
| One Cloud device | Shutdown or reset | Device operations, and the agent trees of the conversations that cannot work without that Cloud |

The Cloud's lifecycle, in the user's shard, reserves device admission before
it asks the machine manager to change the machine.
Tree reservations alone cannot protect a Cloud device shared by multiple
conversations. Normal wake can be joined. Reset interrupts device work and
coordinates the durable disk transition in [managed hosts](../cloud/managed-hosts.md).
It does not silently replay interrupted work.

### How a conversation uses a device

A conversation uses a device in one of three roles, and everything a device's
lifecycle does to conversations follows from the role alone:

| Role | The device is | While a turn runs | When the Cloud resets or stops |
| --- | --- | --- | --- |
| `target` | where the conversation's files and commands are | keeps it awake | The turn is interrupted, the conversation is held, its file transfers and user streams end |
| `provider` | where the process of its model's provider runs | keeps it awake | The turn is interrupted and the conversation is held |
| `attached` | an extra Host the conversation can reach | keeps it awake | Nothing: it runs on its own target |

The `provider` role is derived, never stored: a node of the conversation's
tree, the root or any subagent ([Runtime](../agent/subagents.md#runtime)),
infers with a provider that needs a process on a Host, and the
[placement](../providers/claude-code.md#where-it-runs) says which device that
process runs on. The same placement starts the process, so the role and the
process cannot name different machines. A conversation on a paired device whose
model is Claude Code therefore uses its device as `target` and the user's Cloud
as `provider`. The lifecycle knows roles; it does not know providers.

A reset reaches a conversation through the `target` and `provider` roles, and
no others: those conversations cannot work without the device. A conversation
that merely has the Cloud attached keeps running on its own target: its turn
is not interrupted, its streams stay open, and it can be opened, read and
written throughout. A command it had running on the Cloud ends with the
device, as it would on any Host that went away. Its next Cloud command waits
for the device like any other wake: it waits for the reset to finish and then
runs, and is never refused because a reset is running.

A held conversation is waiting, not failed. While a transition holds a
conversation (a reset, a target change, an archive), everything the user sends
to it waits for the transition and then proceeds: opening it, a message, a
steer, a rename, a pin or a model change, a Host operation. Nothing is answered
with a refusal because the transition is running, so nothing is left for the
user to retry, and the conversation is usable the moment the transition ends.
A wait ends early only when its requester goes away. User streams, the one-shot
user calls admitted like them, and file transfers are the exception the table
names: a transition ends them instead of waiting for them, and the page reopens
its streams by itself afterwards. A request for another transition, a target
change, a detach or an archive, does not wait either: it answers busy, as
[Switch the primary target](#switch-the-primary-target) describes. The page shows
that the conversation is waiting and why (`Cloud is resetting`), from the
Cloud status it already follows.

For example, an idle conversation cannot cause Cloud shutdown while another
conversation is using that same device. A callback remains bound to the job that
originated it throughout the shared-device activity.

## Recovery and persistence

An unavailable paired device or failed Cloud wake produces an operation error.
History remains readable, and the backend does not redirect the operation.

A restart is not a device going away. When the backend starts, the runners
its shutdown disconnected, a running Cloud's among them, connect again by
themselves within seconds: the shutdown lets no Cloud hibernate
([Managed hosts](../cloud/managed-hosts.md)). For the first 30 seconds after the start, a Host
operation for such a device waits for its runner, as an operation waits for
a held conversation, instead of answering `device_offline` or
`host_stopped`; a device still not back after that answers as usual. The
backend knows them as the devices whose last connection ended with its
shutdown. For example, the user presses Reload in a browser tab while Demi
restarts: the page holds the call until it reaches the backend
([The page context](../architecture/plugin-pages.md#the-page-context)), the
backend holds it until the device's runner is back, and the page reloads
with nothing saying that the device was offline.
A page that disconnects does not abort the agent turn; a page that reconnects
synchronizes with the backend transcript.

The agent persists a tool call as executing before dispatch. Recovery distinguishes
these cases:

| Stored state | Recovery |
| --- | --- |
| Result recorded | Preserve the result. |
| Dispatched, result absent | Record an unknown outcome; do not automatically replay side effects. |
| Not dispatched | Continue through the normal resume path. |

A command outlives a connection loss and a backend restart, but not its
runner ([Command lifetime](runner.md#command-lifetime)). The conversation's
database records each running command, with its job and its device
([Command outputs](../backend/storage.md#command-outputs)), and when the
device's runner connects, the backend matches the jobs its hello lists with
those records:

| The runner lists the job as | The command |
| --- | --- |
| Running | Goes on: the backend follows it again where a page shows it, reads the output it missed from the kept output, and the command's handle works again for `demi shell status` and `demi shell stop` |
| Ended | Ends with the status the runner kept, and its whole output is read as for any end; a job the runner stopped after 10 minutes without a connection is lost with that reason, its output kept |
| Not listed | Is lost, with the reason the backend can tell from what it knows of the device: Demi was upgraded and the runner replaced itself, the Cloud restarted, the runner started anew after it ended, or the runner, still the same, never received the job |

A lost command is recorded as ended for that reason, and the node that ran
it is told by its report
([Lost commands](../agent/runtime.md#lost-commands)).

Two kinds of interruption follow ([Upgrades](../delivery/upgrades.md)):

- **The release stays.** A backend that restarts, crashes or loses its
  network stops nothing on the Hosts: runners keep their jobs, Clouds keep
  running, and every command goes on once its runner is back.
- **The release changes.** Each runner replaces itself with the new
  release's, and each Cloud restarts with the new release's programs, so
  their commands end, and each is reported as lost to the upgrade.

Conversation trees remain in backend storage. Working files
remain on their devices. Cloud system and home volumes persist across ordinary
shutdown; reset preserves home. Archiving a conversation does not delete its
session directory or Cloud machine. Workspace deletion requires no referencing
conversations and deletes metadata, not its directory.

Full shell output stays on the execution device until the conversation's Host
resources are released: every Host removes it at the
[conversation release](resource-lifecycle.md#conversation-release), and a
Cloud's reset removes all of it with the system image. Durable results must be
written to persistent directories; the backend transcript retains only the
recorded output view.

## Implementation ownership and checks

The user's shard owns the state this document describes: target transitions;
the conversation's host access, with its file gates and the admission of file
transfers and user streams; each device's runner connection and the
authorization of its callbacks; the conversation idle clock and the
conversation release; and the Cloud's policy and machine transitions (see
[Conversation idle and Host resource release](resource-lifecycle.md)). The edge
moves the bytes of transfers and user streams; admitting them is the
shard's. The agent owns tree admission and per-node context persistence. The
Host adapter carries live execution facts without knowing user policy.
[Crates and packages](../architecture/crates-and-packages.md) defines the
modules and their dependencies.

These rules hold because one shard holds all of a user's conversations,
devices and Cloud: a user is placed on one backend worker, and on one shard in
it. Moving a user to another worker also requires fencing the old worker's
storage and execution authority
([Deployment and user ownership](../backend/backend.md#deployment-and-user-ownership)).

Backend scenarios and agent tests use scripted providers. They cover child
activity during switches, concurrent admission, file uploads, cross-user
refusal, expired jobs, cross-host callbacks, per-node context updates,
and shared-device reset and recovery, and they show that:

- an archive during a Cloud wake refuses the waiting operation once the wake
  ends, and a switch during the wake sends the operation to the new target;
- a Cloud command of a conversation that only has the Cloud attached, sent
  during a reset, runs once the reset ends;
- a rename, a pin or a model change sent while a transition holds the
  conversation completes after the transition;
- an archive ends a running one-shot user call instead of waiting for it;
- backend shutdown with a download open ends the download and still saves the
  Cloud.
