# Conversation idle and Host resource release

One rule decides when Demi reclaims what a conversation uses on a Host: the
conversation's agent has been idle for the idle window. Nothing a tool does on
the Host, such as a conversation browser that is still open or a process that is
still running, postpones reclamation on its own. Every Host, a paired device or
a Cloud, reclaims what a conversation left there one conversation at a time,
through the [conversation release](#conversation-release); a Cloud also stops as
a whole once no conversation using it is active. Cloud policy and durable
machine operations belong to
[Managed hosts](../cloud/managed-hosts.md#lifecycle-and-capacity); Host access
belongs to [Sessions and targets](sessions-and-targets.md#host-operations).

## Activity

A conversation is active while any of these holds:

- A user message has been accepted and its turn has not finished.
- An agent turn is running or waiting for a provider, in the root or any child.
- A shell job of the conversation is running on a Host.
- A Host operation admitted through the conversation's host access is in
  progress. An operation the user does on what runs there, such as closing or
  navigating one of the
  [conversation browser's tabs](../browser/live-view.md#the-tab-methods),
  is admitted and ends at once, so it restarts the window.
- A [user stream](native-runtime.md#user-streams) of the conversation is open,
  such as the [live browser view](../browser/live-view.md): someone is watching
  the conversation's Host, and may operate it. The stream is active from its
  admission until it ends.

Everything else is retention, not activity: open conversation browser tabs,
cookies, a resident native service, a provider's process kept between turns (the
turn that waits for it is the activity; a Host that stops ends the process, and
the next turn starts another), a paired device that stays online, a sidebar
entry, a connected chat, a metadata observer, a look at what runs on the Host
such as listing the conversation browser's tabs, a scheduled future turn that
has not been admitted, a [Host expose](expose.md#lifetime) and its visitors'
traffic. The gates that admit this work are the only source of this fact; no
module keeps a second busy flag.

A look is retention because the page makes it by itself: it lists the
conversation browser's tabs each time it is shown again and after each tool
call, so a listing says nothing about whether anyone uses the Host. An open view
says that someone does, so the Host they watch is not reclaimed under them. The
page closes its view while it is hidden, behind another tab of the user's
browser or in a minimized window, and opens a new one when it is shown again
([Ending a view](../browser/live-view.md#ending-a-view)). So a page left open
on a view keeps its conversation, and its Cloud, active only while it is
visible, when someone may be watching.

## Idle window

The idle window is **1 hour**, one setting, applied to both consequences below.
A conversation becomes idle when its last activity ends; new activity, however
brief, restarts a full window. The user's shard keeps one clock and one timer
per resource, rechecks activity under reserved admission at the deadline, and
never retires work that won admission first.

| Resource | Idle predicate | Consequence |
| --- | --- | --- |
| Cloud device | No conversation [using this device](sessions-and-targets.md#how-a-conversation-uses-a-device), in any role, has been active within the window, and no running job | Save persistent volumes and stop the sandbox; everything inside it ends with the machine, and the device's exposes are destroyed ([A Cloud's idle stop](#a-clouds-idle-stop)) |
| Conversation on a Host | This conversation has not been active within the window | Send the conversation release to each of its Hosts whose runner is connected: a paired device or a running Cloud |

Idle retirement never interrupts active work. A retirement that loses the race
against new activity releases its reservation and recomputes from current facts.

## Conversation release

The conversation release is one generic runner message,
`conversation_release {conversationId}`, sent through the conversation's host
access, in its [lifecycle form](sessions-and-targets.md#lifecycle-access), to a
Host while that Host's runner is connected: a paired device, or a Cloud that
runs. The backend sends it to:

- every connected Host of the conversation when its idle window expires;
- a Host the conversation stops using: the old main device on a target
  switch, an attached device when it is detached;
- every connected Host of the conversation when it is archived.

None of them fails because a Host did not take the release
([A release that fails](#a-release-that-fails)). A release never wakes a
stopped Cloud: the stop has already ended everything a release ends.

The runner forwards the release to every resident native service on that device
as the generic [conversation release operation](native-runtime.md#conversation-scoped-state);
each service ends whatever it holds for that conversation and acknowledges. The
conversation browser is one such holder: it closes that conversation's Chrome
and removes its profile. The runner acknowledges once the services have
answered. It holds no files of the conversation for a release to remove: a job's
directory goes as soon as the backend has read the job's end, and what Demi
keeps of a command's output, the backend has stored
([Pipes and output](runner.md#pipes-and-output)).

No job of the conversation runs when its release arrives: the backend sends a
release of a conversation of the device's owner only while it holds the
conversation's file gate, and every job runs inside a lease of that gate
([Host operations](sessions-and-targets.md#host-operations)); a conversation
of anyone else runs nothing on the device.

Repeating a release is harmless. A release never starts a service that is not
running. When the device is offline or the Cloud is stopped, the connection
loss or the stop has already ended the services' state, so a release the
device did not hear leaves nothing behind, and none is sent when it connects
again.

Forking a conversation creates a new conversation and releases nothing. A runner
shutdown or connection loss ends every conversation's service state on that
device through the native service shutdown contract.

### A Cloud's idle stop

A Cloud stops as a whole once no conversation using it has been active within
the window ([Idle window](#idle-window)). The stop sends no releases: what a
release would end, such as a conversation's Chrome, ends with the machine, and
no command's output is left on its disk to save with it
([Pipes and output](runner.md#pipes-and-output)). The conversations' own idle
watches find the Cloud stopped and release the conversations on their other
Hosts, if they have any. A Cloud that stops for another reason, at its
lifetime cap, for a reset, at the backend's shutdown or because it died, is
the same.

### A release that fails

A user archives a conversation, and the laptop it ran on loses its network in
the middle of the release. The archive succeeds: the lost connection has ended
the conversation's Chrome on the laptop, as every connection loss ends the
services' state.

A release fails when the Host's runner loses its connection before it
answers, when it answers that a service could not end what it held, or when
it does not answer within six minutes. A paired device loses its connection
when it sleeps or its network drops, and a Cloud when it stops. The backend
logs a failed release with the Host and the reason, and whatever sent it goes
on as if the Host had taken it:

| Sender | After a failed release |
| --- | --- |
| Archive | The conversation is archived |
| Target switch | The conversation moves; the device it left stays attached |
| Detach | The device is detached |
| The conversation's idle watch | The watch ends, as after a release |

None of them fails or sends the release again, because nothing is left for
it. A runner that lost its connection has ended every service's state. A
runner that answered that a service failed has retired that service, which
ends what it held
([Conversation-scoped state](native-runtime.md#conversation-scoped-state)).
A runner that answers too late finishes the release all the same. An archive,
a target switch and a detach are the user's changes to their own
conversation: failing one because a Host's cleanup failed would leave the user
a refusal to retry that changes nothing on the Host.

## Acceptance

Verify without real models, with scripted activity: the idle watch alone on a
paused clock, and the release through a user's shard, which waits on the
database, in real time with a short window
([Tests and time](../architecture/concurrency.md#tests-and-time)):

| Situation | Required result |
| --- | --- |
| Activity arrives just before the deadline | Exactly one of activity or retirement wins; no live work is stopped as idle |
| A conversation with open conversation browser tabs idles for the window on a paired device | The device receives one release; its Chrome and profile are gone; the device and runner remain available |
| Commands of two conversations end on one paired device | Each job's directory is gone once the backend has read the job's end; nothing of either conversation's commands stays on the device |
| A paired device loses its connection while a command runs | Its runner removes the job's directory; the command's stored output is what the backend received |
| Two conversations used a running Cloud, and one idles for the window while the other stays active | The idle one's Chrome and profile are gone; the machine keeps running |
| The last conversations using a Cloud idle for the window | The Cloud stops without a release; the generation it saved holds no job directory |
| A conversation is archived while its Cloud is stopped, and the Cloud wakes later | The archive does not wake the Cloud; after the wake the Cloud holds nothing of the conversation |
| A paired device loses its connection while an archive's release waits for its answer | The archive succeeds; its Chrome for the conversation ended with the connection, and no release is sent when it connects again |
| A running job or a waiting child turn exists at the deadline | Nothing is retired; the window restarts when the activity ends |
| A live browser view stays open with no agent activity, while the page lists the conversation browser's tabs | Nothing is retired while the view is open, and the window starts when it closes; the listings restart nothing |
| Target switch or archive while a timer is pending | One release to the old device; a stale timer cannot release the new binding |
| Connection loss or backend shutdown | No idle watch or timer outlives it; no release attempt against a lost device |
