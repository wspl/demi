# Turns

What the user sees and does while an agent works: steering or queueing a message, recovering a turn that did not finish, following the agent's work, and notifications.

## Steer or queue

While the agent works, a message the user sends either steers the running
turn or waits in the queue, one or the other, and stays what it was sent as:

- **Steer**: the agent reads the message within the running turn, as soon as
  the call it is waiting on returns. Until then the message shows as pending
  above the composer.
- **Queue**: the message waits for the turn to end and then starts a turn of
  its own.

A message shows what it was sent as from the moment it is sent, since the
page chose the way: a steer shows at once with *Will be sent to the agent at
its next step*, Send Now and Recall, and a queued message with its own
line, while the backend has not confirmed it yet, in the same row and at the
same height it keeps once confirmed. Send Now or Recall pressed before the
confirmation is kept as the user's wish and carried out once the message is
confirmed ([Responding to the user](web-application.md#responding-to-the-user)).
Recall takes the message back, as Outlook recalls a sent mail: it leaves the row at
once, confirmed or not, and its request follows. When the composer is empty,
the message goes back into it, its text and its files, the caret at its
end, ready to change and send again; when the composer holds a draft, the
draft is kept as it is and the message is not put back, and a toast,
*Message Recalled*, offers Undo, which sends it again the way it was sent. A
refusal, as when the agent read the steer meanwhile, brings the row back
where it was with a toast, and takes back out of the composer what it had put
there, unless the user changed it since. A message taken back before it was
delivered, as one whose files are still uploading, goes back into an empty
composer with its files as they were; beside a draft it stays as an
undelivered message with Retry, since files that never finished uploading
have nothing stored to send again. A message can be sent while the one before
it is still on its way: it goes the way the composer chooses, steer or
queue, after the one before, never ahead of it.

For example, the agent is running the test suite. The user types "skip the
e2e tests" and presses Enter: with Steer chosen, the agent reads it when the
test command's call returns and goes on in the same turn; with Queue chosen,
it reads it after it has finished and answered.

Settings › General chooses what Enter does while the agent works; Steer is the
default, as in Claude Code and Codex. ⌘/Ctrl+Enter, or ⌘/Ctrl-click on the
send button, sends that one message the other way. While the agent works, the
send button's tooltip names what it does and the key for the other way. While
nothing runs, both send the message at once. A steer the session refuses
because its turn is ending is sent as a message instead, so it is answered
next and the user sees no error.

A pending steer or a queued message can be sent now with Send Now on its row:
the agent does not wait for its running call or for the end of its turn. A
command the agent is running moves to the background and keeps running, and
the agent is told so; a reply it is writing is cut where it is. A steer sent
now enters the turn at once, which goes on; a queued message sent now ends
the turn there and starts its own
([Send now](../agent/runtime.md#send-now)). Stop is different: it stops the
turn and the commands it is running.

## Recovering an unfinished turn

A turn can end without finishing in two ways, and the agent can go on from
either without the user typing anything.

- **Something broke.** The provider refuses the request after the automatic
  retries give up, a compaction's summary request included, or the agent session is shut down under a running turn: a
  backend restart, a crash, a release. Every one of these is an error: the
  transcript gets an error record that says what happened. A provider failure
  carries the provider's own words; a failure of Demi's own carries Demi's
  fact: "The agent session was shut down while this turn was running." A graceful
  shutdown writes the record as it ends the turn; after a crash the restored
  conversation writes it when it is next opened. No turn ends unfinished
  without either such a record or the user's own Stop. A turn a graceful
  shutdown interrupted, such as one a deployment or an upgrade ended,
  goes on by itself once the backend is back, unless the user turned that
  off in Settings › General
  ([The unfinished turn](../agent/failures-and-recovery.md#the-unfinished-turn));
  its record stays in the transcript, followed by the turn's progress.

  A Host that goes offline does not stop a turn while it runs. The operation
  that needed it fails with the runner's offline error, worded for the agent
  ([Host operations](../execution/sessions-and-targets.md#host-operations)),
  the tool call shows that error, and the agent decides: it retries, or it
  ends its turn and says what it could not do. A turn that ends with work
  left undone because its Host was away is unfinished, as one that broke
  is: when the root's turn ends and a call of it failed because its Host
  was offline, with no later call of the turn run on that Host, the
  transcript gets an error record of Demi's fact, "MacBook Pro went offline,
  so this turn could not finish its work.", and the dock offers **Resume**,
  which stays disabled, *MacBook Pro is still offline*, while the Host the
  conversation runs on is an offline device: until the device is back, or
  the user moves the conversation to a Host that is online. Nothing wakes the agent by itself when the device returns, so a user
  who was away is not surprised by it starting work again; the user
  resumes, and the agent's next request says the device is back. Above the
  composer the device's offline card says how to start its runner and
  offers **Move to Another Host…**, the Run On menu's online Hosts, which
  moves the conversation as the menu does once the tree is idle
  ([Switch the primary target](../execution/sessions-and-targets.md#switch-the-primary-target)).
- **The user stopped it.** Stop is a decision, not a failure, and the
  transcript marks the turn as stopped.

The transcript says what happened; the place to act is above the composer.
The error record leads with what its source said, keeps the facts and Copy,
and carries no button. A failure that names when it can succeed again, such
as a usage limit that resets at 09:00, also says that time and carries the
one switch that belongs to it: *Resume at 09:00*, a checkbox that is the
user's preference to resume such a turn by itself
([The unfinished turn](../agent/failures-and-recovery.md#the-unfinished-turn)),
the same as Settings › General's *Resume after a usage limit resets*. It is
a preference, not a choice for this record: ticked on one record, it holds
for every later one and in Settings, and the record whose time has passed
shows it no more. The fact and the checkbox name the same time, rounded up
to the minute so it is never before the reset: a reset at 7:03:55 PM reads
*Resets at 7:04 PM* and *Resume at 7:04 PM*, with the weekday when it is not
today; on a phone the fact wraps rather than cutting the time.
One recovery control sits in the dock, directly over the input, where the
user's next action already is:

```text
  ... transcript ...
  [x] The provider request failed
      Insufficient balance. Manage your billing here: ...
      HTTP 401 · req_01J8...

  +--------------------------------------------------------------+
  | MacBook Pro is offline.               [Move to Another Host…] |  <- a card
  +--------------------------------------------------------------+
  [ > Resume ]   [ 2 Running ]  [ 1 Agent ]        <- the dock's chips
  +--------------------------------------------------------------+
  | Ask Demi...                                  GLM-5.3-Flash   |
  +--------------------------------------------------------------+
```

The chips sit directly on the input, and nothing comes between them: a card
above the composer, such as a device's offline card, a permission request
or a notice, stands above the chips, so the control the user reaches for
next stays where the hand already is.

The Agents and Running chips each open a window over the session, as an
editor's bottom panel opens. Its top edge resizes it: dragged, the window
grows or shrinks with the pointer; double-clicked, it returns to its usual
height; focused, the arrow keys move it. Its height is kept as a share of
the session's height, so a window made tall in a large browser window
stays tall, and never larger than the room, in a small one; it never
shrinks below its header, its bar and a few lines of transcript, nor grows
over the session's own header. The two windows keep one height between
them, as an editor's bottom panel keeps one for each of its views. On a
phone a window takes the whole screen and has no edge to drag. Its windows, and the windows they open, such as
the media viewer over a screenshot in a subagent's transcript or the
terminal of a running command, stand in the order they opened, the newest in
front. A press outside every window, on the transcript, the sidebar, the composer,
the dock's empty space or the cards above it, closes the newest window
alone, as Escape does, and still does what it does there, so a click on a
link closes the window and follows the link, and a click in the message
puts the caret there. A second press closes the next one. A press in a
window closes nothing. Typing does not close a window: a user whose caret
is already in the message types while watching it. A
control that opens or changes one of these windows, a thumbnail, an
attachment, an agent link or a receipt's sender, acts as that window's own
control rather than as a press outside it: a second thumbnail shows its
picture in the open viewer, and an agent link to the tab the window shows
leaves it as it is. A window also closes by its chip or its
close control. For example, the user opens *2 Agents* and clicks a
screenshot in the child's transcript: the viewer opens over the Agents
window; a click on the transcript closes the viewer and leaves the Agents
window open on the child; another click closes it.

The recovery control's label follows the cause, because the user's expectation differs:

| How the turn ended | Control | What the user expects |
| --- | --- | --- |
| An error ended it: a provider failure, or a shutdown or crash under the turn | **Resume** | The same step again: nothing was decided, something broke. |
| The user pressed Stop | **Continue** | The agent goes on with the work the user cut short. |
| It finished | none | |

Both are the same operation, the session's `resume`: it unwinds to the turn's
resume point, keeps everything that already left the process, and infers
again from there. It adds no user message and never reruns a completed tool
effect. The label is presentation only.

The control is offered when the last turn is unfinished and the conversation
is idle, not archived, has a usable provider, has its history loaded, and no
message edit is open. It leaves when recovery starts, and for good once a
newer turn exists: sending a message is the other way forward, and the earlier
turn is then history. An action that began no turn offers nothing: a
Compact after a finished answer whose summary request failed, or that the
user stopped. A compaction's divider does not end a turn either: behind it,
an unfinished turn keeps its Resume or Continue
([The unfinished turn](../agent/failures-and-recovery.md#the-unfinished-turn)).

From the moment the user sends a message until the answer begins, the
transcript's tail row says **Requesting**, with how long it has waited since
the send. The clock is the conversation's, not the page's: a page opened or
reloaded during the wait, and a subagent's view shown during its wait, count
from when the wait began
([Waiting for the model](../agent/rendering.md#waiting-for-the-model)), so a
wait that has lasted long never reads as one that just began. The message shows at once as it will stay, and delivering it to
Demi is part of the same wait: there is no separate sending state, since
which part of the system holds the message is no concern of the person
waiting, and a row that changed its word when Demi confirmed the message
would show two names for one wait. The word says what the wait is for, an
answer: a slow model must never read as something broken in Demi. Only a
delivery that fails replaces the row, with the message marked as not
delivered and a Retry, which resends it and shows Requesting again. Recovery shows the
same row from the moment the control is used; there is no separate resuming
state. The row says Requesting, and nothing else, for as long as the provider
is being asked, a CLI installing on the Cloud for it included: while the agent retries a failed request on its own (a rate
limit or an overload, within the retry policy of
[Failures and recovery](../agent/failures-and-recovery.md)) the row and
its clock stay as they are. Each attempt is a request, and a row that changed
its word with every attempt would flicker between two names for one wait. The
retries have no control; the Resume control appears only after they give up.

While a turn runs, the transcript's end always shows that work goes on:
reasoning or text as it streams, a call being written or running
([Calls being written](../agent/rendering.md#calls-being-written)), or the
Requesting row. Text the model has finished, with nothing after it yet, is
followed by the Requesting row, its clock counting from the text's end: a
vendor that sends its calls only whole leaves that gap between a sentence
and the call that follows it.

Two other retries are not this and keep their own places: reloading a history
that failed to load, and resending a message whose delivery is unconfirmed.
They recover reading and delivery, not a turn.

## Following the agent's work

The user sends "run the tests" while reading the end of a conversation. The
message appears at the bottom of the view in the frame it is drawn, and so
does each row after it: the Requesting row, the agent's first step, its
answer, the answer's Copy and Fork. Nothing appears below the view first and
then jumps into it, and nothing scrolls to it a frame later; a delay of a
few milliseconds reads as the list twitching.

- **At the end.** The reader is at the end while the end of the list is in
  view or less than 16 px below it. While at the end, whatever grows the
  list at its end, a row, a row's growth, a footer, the Requesting row or a
  failure notice, shows at the bottom of the view in the frame that draws
  it. A reader who scrolled up stays where they are, and the
  scroll-to-bottom button offers the way back.
- **The list only grows.** A row that changes form keeps its place and
  changes in the same frame: the message being sent becomes the message the
  server confirmed, a step rolls over the one before, a call being written
  becomes its call, the Requesting row becomes the step it waited for. The
  list never draws a frame in which it is shorter, only to grow back. Only
  what the user does shrinks it: folding a row, editing a message, which
  replaces what follows it, or deleting.
- **What replaces a row takes its place.** Whenever something the
  conversation shows gives way to what comes next without the user removing
  it, the new state rolls over the old one in the same row, with the roll a
  step makes over the one before; a row never goes while another appears
  beside it. One rule, for every such change: the message being sent and the
  message confirmed; Connecting, Requesting and the step that arrives;
  thinking without text and the text or step that follows it; a call being
  written and its call; a message that could not be delivered and the turn
  its Retry starts; a conversation that failed to open and the tail row once
  it opens; *Compacting* and the compaction's divider; a run's live face and
  its summary. For example, the
  user presses Continue after a Stop: the *Stopped* row rolls into
  *Requesting* in that place, and Requesting into the turn's next step; Resume
  after an error rolls the error record the same way, and when the turn's
  first step arrives it joins the run above, and the Requesting row gives way
  ([Work groups](../agent/rendering.md#work-groups)). A change the backend
  refuses rolls the row back to what it said. A taller row that rolls into a
  shorter one, as an error record into Requesting, narrows over the roll:
  the user's Continue, Resume or Retry asked for it, so the list may shrink
  then, and only then. A row moves only when the
  message it shows moves, as a steer the turn took leaves the waiting messages
  for its place in the transcript.
- **Messages that wait stay out of the turn's flow.** A pending steer and a
  queued message wait below the turn, and what the turn is doing is read from
  the transcript alone: the tail row, Requesting or Connecting, stands right
  after the transcript, above the messages that wait, and only when the
  transcript's end waits for the model. For example, the agent is writing a
  command, its row shimmering *Install the test runner*, and the user sends a
  steer: the steer shows under that row with *Will be sent to the agent at
  its next step*, and nothing shows under the steer, since the run's row
  already says the agent works.
- **A turn's end is one change.** The turn's last text and the conversation
  going idle reach the page together, so the page never draws a frame between
  them in which finished text, with nothing after it, is followed by
  Requesting.
- **A steer or queued message that will start the turn** is drawn as the
  message it becomes: when nothing runs and the first message on its way was
  sent as a steer or to the queue, as when the message before it was not
  delivered, its row reads as the user's next message, with no line of its
  way, from the moment the page knows it starts the turn.
- **A message that was not delivered** waits at the end, after the turn a
  later message started, with Retry: it is the message still to send, and
  Retry sends it next.
- **One size for the reading.** The agent's text, the user's messages and
  the transcript's rows, a step, a notice, a receipt, and a receipt's message
  an opened row shows, read at one size, the size the user picks in Settings
  › Appearance, 15 pixels unless they pick another; the rows stand apart by
  their color, not their size. A thinking's text, which the reader skims
  rather than reads, stands one pixel smaller, with looser lines, and follows
  the same setting. Labels under a
  message, its time or a steer's line, stay small.
- **The message being sent** is drawn as the message it will become, at its
  height: whether it is still on its way shows in the Requesting row, and a
  lost connection in the Connecting row or the connection banner, never as
  a line under the bubble that later goes.
- **Copy and Fork** close the text that ends the agent's reply to a message:
  the text after which the user's next message, the user's Stop, or nothing
  while the conversation is idle comes. Text after which the agent went on
  working, with a step or another agent's message, is not the end of its
  reply, and keeps no footer even when the user writes next. A message being
  sent counts as the user's next message from the moment it is sent, so the
  footer of the answer before it stays while the message is delivered.

## Notifications

A user sends a long task and switches to another app. When the turn ends,
the system shows a notification from the browser with the conversation's
title and the start of the answer; clicking it brings the Demi tab forward
with that conversation open.

Notifications are off until the user turns them on in Settings ›
Notifications. The setting belongs to the browser, not the account, because
a browser grants a site permission to notify on that browser alone; the page
keeps it in the browser's storage. Turning on Browser notifications asks the
browser for permission. When the browser refuses, the switch goes back off and
the row says that the browser blocks notifications for this site and that its
site settings allow them again. Under it the user chooses what notifies, each
on by default:

| Switch | Notifies when | The notification says |
| --- | --- | --- |
| A turn finishes | A turn of the conversation's root ends with an answer, and the conversation no longer [runs](web-api.md#sidebar-mutations-and-read-state): its subagents are done too | The title, and the start of the answer as plain text, cut to 120 characters |
| A turn fails | A turn of the root ends with an error, which offers Resume | The title, and that the turn stopped with an error |
| Demi needs permission | The conversation asks for a [permission](../agent/permissions.md) | The title, and what the request asks |

A notification shows only while its conversation is not in front of the
user: the page is hidden or not focused, or it shows another conversation. A
turn the user stopped notifies nothing, since the user was there. The pages
learn of these events from their synchronization channel, a turn's end from
the conversation's summary
([Sidebar mutations and read state](web-api.md#sidebar-mutations-and-read-state)), so notifications
come while any Demi page of the browser is open, and none come when none is,
which would need push notifications. Each notification is tagged with its
conversation and turn, or its permission request, so several open pages show
it once. The system's own notification sound plays as the system's settings
say; Demi adds no sound of its own.

The instruction tells the model to produce a title, never an answer: one line
that fits where it is shown, in the language the user writes in, where names
such as packages, files and commands count as no language, so an English
message full of package names still gets an English title. The instruction
describes the place rather than listing a length per language: one line of a
narrow sidebar about 24 columns wide, where a Latin character takes one column
and a Chinese, Japanese or Korean character two, and the overflow is cut off.
It holds no examples: a model copies an example's language and shape, as a model
gives an English request a half-Chinese title when the instruction's
examples end in Chinese ones. It also asks for natural grammar,
exact technical terms, file names, numbers and error codes kept, no tool
names, no leading "the" or "my", and something meaningful even for a greeting.

The page learns the generated title the way it learns a rename made
elsewhere, from the conversation's summary on its
[synchronization channel](web-api.md#page-synchronization); there is no title
event of its own.

Not included: asking the model for a new title later, retitling by itself as
a conversation grows, and titling from a different, cheaper model. The last is a
natural extension once instance or user configuration names such a model.

Rationale: a request beside the turn, rather than a copy of the session the
way [compaction](../agent/compaction.md#compaction) summarizes, keeps the
agent's system prompt, tools and history out of a job that needs one message,
and keeps the title independent of whether the first turn succeeds.
