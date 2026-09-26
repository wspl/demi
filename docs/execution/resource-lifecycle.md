# Conversation idle and Host resource release

One rule decides when Demi reclaims what a conversation uses on a Host: the
conversation's agent has been idle for the idle window. Nothing a tool does on
the Host, such as a browser that is still open or a process that is still
running, postpones reclamation on its own. Cloud reclaims at the device level;
a paired device reclaims the conversation's resources one conversation at a
time. Cloud policy and durable machine operations belong to
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
| Cloud device | No conversation [using this device](sessions-and-targets.md#how-a-conversation-uses-a-device), in any role, has been active within the window, and no running job | Save persistent volumes and stop the sandbox; everything inside it ends with the machine, and the device's exposes are destroyed |
| Conversation on a paired device | This conversation has not been active within the window | Send the conversation release to that device |

Idle retirement never interrupts active work. A retirement that loses the race
against new activity releases its reservation and recomputes from current facts.

## Conversation release

The conversation release is one generic runner message,
`conversation_release {conversationId}`, sent through the conversation's host
access, in its [lifecycle form](sessions-and-targets.md#lifecycle-access), to a
paired device the conversation is bound to, main or attached. The backend sends
it to:

- every connected paired device of the conversation when its idle window expires;
- a paired device the conversation stops using: the old main device on a target
  switch, an attached device when it is detached;
- every connected paired device of the conversation when it is archived.

A Cloud never receives a release: it reclaims at the device level, when it
stops.

The runner forwards the release to every resident native service on that device
as the generic [conversation release operation](native-runtime.md#conversation-scoped-state);
each service ends whatever it holds for that conversation and acknowledges. The
browser is one such holder: it closes that conversation's Chrome and removes its
profile. Repeating a release is harmless. A release never starts a service that
is not running; when the device is offline, there is nothing to release and the
connection loss has already ended the state.

Forking a conversation creates a new conversation and releases nothing. A runner
shutdown or connection loss ends every conversation's state on that device
through the native service shutdown contract.

## Acceptance

Verify without real models, with scripted activity: the idle watch alone on a
paused clock, and the release through a user's shard, which waits on the
database, in real time with a short window
([Tests and time](../architecture/concurrency.md#tests-and-time)):

| Situation | Required result |
| --- | --- |
| Activity arrives just before the deadline | Exactly one of activity or retirement wins; no live work is stopped as idle |
| A conversation with open browser tabs idles for the window on a paired device | The device receives one release; its Chrome and profile are gone; the device and runner remain available |
| A conversation idles for the window on Cloud | The machine stops; no release message is sent to it |
| A running job or a waiting child turn exists at the deadline | Nothing is retired; the window restarts when the activity ends |
| A live browser view stays open with no agent activity, while the page lists the browser's tabs | Nothing is retired while the view is open, and the window starts when it closes; the listings restart nothing |
| Target switch or archive while a timer is pending | One release to the old device; a stale timer cannot release the new binding |
| Connection loss or backend shutdown | No idle watch or timer outlives it; no release attempt against a lost device |
