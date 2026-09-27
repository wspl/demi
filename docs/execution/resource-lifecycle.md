# Conversation idle and Host resource release

One rule decides when Demi reclaims what a conversation uses on a Host: the
conversation's agent has been idle for the idle window. Nothing a tool does on
the Host, such as a browser that is still open or a process that is still
running, postpones reclamation on its own. Every Host, a paired device or a
Cloud, reclaims what a conversation left there one conversation at a time,
through the [conversation release](#conversation-release); a Cloud also stops
as a whole once no conversation using it is active. Cloud policy and durable
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
  [conversation browser's tabs](../product/web-api.md#conversation-browser-tabs),
  is admitted and ends at once, so it restarts the window.
- A [user stream](native-runtime.md#user-streams) of the conversation is open,
  such as the [live browser view](../browser/live-view.md): someone is watching
  the conversation's Host, and may operate it. The stream is active from its
  admission until it ends.

Everything else is retention, not activity: open browser tabs, cookies, a
resident native service, a provider's process kept between turns (the turn that
waits for it is the activity; a Host that stops ends the process, and the next
turn starts another), a paired device that stays online, a sidebar entry, a
connected chat, a metadata observer, a look at what runs on the Host such as
listing the conversation browser's tabs, a scheduled future turn that has not
been admitted, a [Host expose](expose.md#lifetime) and its visitors' traffic.
The gates that admit this work are the only source of this fact; no module
keeps a second busy flag.

A look is retention because the page makes it by itself: it lists the
browser's tabs each time it is shown again and after each tool call, so a
listing says nothing about whether anyone uses the Host. An open view says
that someone does, so the Host they watch is not reclaimed under them. The
page closes its view while it is hidden, behind another browser tab or in a
minimized window, and opens a new one when it is shown again
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
| Cloud device | No conversation [using this device](sessions-and-targets.md#how-a-conversation-uses-a-device), in any role, has been active within the window, and no running job | Send the conversation releases that fall due with the stop ([A Cloud's idle stop](#a-clouds-idle-stop)), then save persistent volumes and stop the sandbox; everything inside it ends with the machine, and the device's exposes are destroyed |
| Conversation on a Host | This conversation has not been active within the window | Send the conversation release to each of its Hosts whose runner is connected: a paired device or a running Cloud |

Idle retirement never interrupts active work. A retirement that loses the race
against new activity releases its reservation and recomputes from current facts.

## Conversation release

The conversation release is one generic runner message,
`conversation_release {conversationId}`, sent through the conversation's host
access, in its [lifecycle form](sessions-and-targets.md#lifecycle-access), to a
Host the conversation is bound to, main or attached, while that Host's runner
is connected: a paired device, or a Cloud that runs. The backend sends it to:

- every connected Host of the conversation when its idle window expires;
- a Host the conversation stops using: the old main device on a target
  switch, an attached device when it is detached;
- every connected Host of the conversation when it is archived.

A release never wakes a stopped Cloud. A Cloud hears the release while it
runs, as a paired device does while it is connected; a release that falls due
while the Cloud is stopped reaches it after its next wake
([A release the device missed](#a-release-the-device-missed)).

The runner forwards the release to every resident native service on that device
as the generic [conversation release operation](native-runtime.md#conversation-scoped-state);
each service ends whatever it holds for that conversation and acknowledges. The
browser is one such holder: it closes that conversation's Chrome and removes its
profile. The runner itself holds the conversation's job directories: each shell
job of the conversation keeps its whole output there, and a tool result names
such a file, for example `<binary stdout: 412000 bytes; raw bytes at <path>>`
([Pipes and output](runner.md#pipes-and-output)). The directories are on the
Host's disk: in a paired device's installation state, and on a Cloud's system
image, which a stop keeps and a
[system reset](../cloud/managed-hosts.md#system-reset) replaces
([Images](../cloud/managed-hosts.md#images) says why there). The release
removes that conversation's directories and no other's, so a path a transcript
names there is invalid afterwards. The runner acknowledges once the services
have answered and the directories are gone. A directory it cannot remove goes
to the [Host log](runner.md#host-log) with the reason, and the next release of
the conversation tries again; the release still succeeds, since nothing of the
conversation runs there any more.

No job of the conversation runs when its release arrives: the backend sends a
release of a conversation of the device's owner only while it holds the
conversation's file gate, and every job runs inside a lease of that gate
([Host operations](sessions-and-targets.md#host-operations)); a conversation
of anyone else runs nothing on the device. The runner still keeps the
directory of any job it runs, whatever the release names.

Repeating a release is harmless. A release never starts a service that is not
running. When the device is offline or the Cloud is stopped, the connection
loss or the stop has already ended the services' state, but not the job
directories, which stay on the Host's disk until it hears the release after it
connects again ([A release the device missed](#a-release-the-device-missed)).

Forking a conversation creates a new conversation and releases nothing. A runner
shutdown or connection loss ends every conversation's service state on that
device through the native service shutdown contract; the job directories stay.

### A Cloud's idle stop

A Cloud stops as a whole once no conversation using it has been active within
the window ([Idle window](#idle-window)). So when the last conversations using
it go idle, their releases and the stop fall due together. For example, a user
works in one conversation on the Cloud until noon and then leaves. At 13:00
both the conversation's window and the Cloud's have passed: the conversation's
idle watch is due to send the release, and the Cloud's idle watch is due to
stop the machine. Which of the two acts first would be chance. A stop that
came first would cut the release off: the conversation's output would stay on
the Cloud's disk and go into the generation the stop saves, and its release
would wait for the Cloud's next wake.

So the idle stop sends those releases first. Before it saves and stops the
machine, it sends the release of each conversation that has the Cloud as its
main or attached Host and whose idle watch has not released it yet; every
conversation using the Cloud has been idle for the window by then. Each
release is sent under the conversation's file gate, as every release is: the
stop already holds the conversations that cannot work without the Cloud, and
it reserves the gate of one that only has the Cloud attached for its release,
passing over one that work or a transition holds at that moment. A release
that fails is logged and the stop goes on; the Cloud's next `hello` names what
it left. In the example, the stop closes the conversation's Chrome and
removes its job directories, then saves the machine without them and stops
it. The conversation's own idle watch finds the Cloud stopped and releases the
conversation on its other Hosts, if it has any.

Only the idle stop sends releases. A Cloud that stops for another reason sends
none: at its lifetime cap or at the backend's shutdown, the conversations using
it have not all been idle for the window; a reset removes every job directory
with the system image anyway; and a sandbox that died hears nothing. The job
output such a stop leaves on the system image is released after the next wake,
as a paired device's missed release is.

### A release the device missed

A laptop that sleeps through a conversation's idle deadline misses the
release, since the backend sends a release only to a connected runner, and so
does a Cloud that is stopped when a release falls due, such as when a
conversation is archived. Without a second chance, that conversation's job
output would stay on the device for good. So the runner's `hello`, a Cloud's
as a paired device's, names every conversation it holds job directories for,
and the backend, once it has bound the connection, answers for each:

- a conversation the device's owner does not have, or one that is archived or
  no longer bound to the device: the release, at once, under the
  conversation's file gate when it is the owner's;
- any other: the conversation's idle watch starts unless it runs
  ([Idle window](#idle-window)), so an idle conversation hears the release one
  window later.

## Acceptance

Verify without real models, with scripted activity: the idle watch alone on a
paused clock, and the release through a user's shard, which waits on the
database, in real time with a short window
([Tests and time](../architecture/concurrency.md#tests-and-time)):

| Situation | Required result |
| --- | --- |
| Activity arrives just before the deadline | Exactly one of activity or retirement wins; no live work is stopped as idle |
| A conversation with open browser tabs idles for the window on a paired device | The device receives one release; its Chrome and profile are gone; the device and runner remain available |
| Two conversations ran shell jobs on one paired device, and one of them is released | That conversation's job directories are gone and the other's remain |
| A paired device was offline at a conversation's idle deadline and connects again | Its `hello` names the conversation, which hears the release one window later, or at once when it is archived or no longer bound to the device |
| Two conversations used a running Cloud, and one idles for the window while the other stays active | The idle one's job directories are gone, and so are its Chrome and profile; the machine keeps running |
| The last conversations using a Cloud idle for the window | The Cloud hears their releases, then stops; their job directories are gone from the generation it saved |
| A conversation is archived while its Cloud is stopped, and the Cloud wakes later | The archive does not wake the Cloud; the Cloud's `hello` after the wake names the conversation, which hears the release at once |
| A running job or a waiting child turn exists at the deadline | Nothing is retired; the window restarts when the activity ends |
| A live browser view stays open with no agent activity, while the page lists the browser's tabs | Nothing is retired while the view is open, and the window starts when it closes; the listings restart nothing |
| Target switch or archive while a timer is pending | One release to the old device; a stale timer cannot release the new binding |
| Connection loss or backend shutdown | No idle watch or timer outlives it; no release attempt against a lost device |
