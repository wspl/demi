# Product design

Demi is a web application for conversations that can run commands on Cloud
or the user's connected machines. Conversation history belongs to the backend;
project files belong to execution devices. Changing a conversation's target
changes where subsequent work runs without moving its history or copying files.

Demi does not support MCP servers: a user cannot add one, and no conversation
reaches one. Demi speaks MCP only inside its Claude Code provider, to serve
its own tools to the CLI it drives
([The SDK MCP channel](../providers/claude-code.md#the-sdk-mcp-channel)).

## Instance mode: shared vs isolated

`DEMI_INSTANCE_MODE` selects whose provider entries a user infers with. The
product cannot change this deployment setting. Every entry belongs to a user;
a shared instance has no separate instance-owned kind of entry. Its providers
are the master's, and only the master configures them. Entries a user made
under the other mode stay theirs and are simply not the ones the mode selects.

| Mode | Provider configuration | Model use | Usage visibility |
|---|---|---|---|
| Shared | The master manages their own entries | All users use the master's entries | Each user sees their own usage; admins can see instance usage by user |
| Isolated | Each user manages their own entries | Each user uses their own entries | Each user sees their own usage |

Conversation, device, workspace, and attachment ownership remains per user in
both modes. Administrator status does not grant access to another user's
conversations or files. Shared usage reporting and account administration are
explicit exceptions to owner-only product records.

A provider is a configured entry with a stable ID and label. Model selection uses
`(providerId, modelId)`, so two entries can offer the same vendor model under
different accounts or endpoints. Subscription entries can contain multiple
accounts with one selected account. Credential rules belong to
[Providers](../providers/providers.md), and catalog rules to
[Models](../providers/models.md).

## User system

Accounts use email and password; nickname is a separate display value. The first
account is created by instance setup. Administrators create subsequent accounts;
there is no public registration or password-recovery flow.

| Role | Account administration |
|---|---|
| Master | Creates admins and users; resets passwords of admins and users |
| Admin | Creates users; resets user passwords |
| User | No administration of other accounts |

A role can act only on lower roles. No role can reset a peer's password through
the admin endpoint. Everyone can change their own password by providing the
current password, edit their nickname, and change email after verifying a code
sent to the new address. Setup and account creation do not require email delivery;
email change does, and reports an unavailable mail service explicitly.

An account can also sign in with a passkey, beside its password, which every
account keeps. For example, Ana adds a passkey in Settings › Account: she
enters her password, her browser offers to save the passkey in its password
manager, such as Google Password Manager in Chrome or iCloud Keychain in
Safari, and from then on any device that manager syncs to signs her in with
Touch ID or Face ID. The sign-in page offers her passkeys in the email
field's autofill and behind Sign In with Passkey, neither of which needs her
email first; a manager holding passkeys of several accounts lists each by
its email. An account has any number of passkeys, one per manager or
security key, and lists them in Settings › Account with the manager's name,
such as iCloud Keychain, whether it syncs (Synced) or stays on one device
(This Device Only), and when it was added and last used; the user renames
or removes one there, and changes the password there. Adding a passkey
always takes the account's password, so a browser left signed in cannot add
one; instance setup offers the new master account to add one right after
its password, which it has just entered. An administrator's password reset
leaves the account's passkeys as they are.

Passkeys belong to the host name of the backend's public URL, such as
`demi.example`: a backend reached at another name, or by an IP address,
for which browsers make no passkeys, offers none.

Handoff carries what the user is doing from a signed-in browser to a phone,
by a code, as Handoff carries work between a Mac and an iPhone. For example,
Ana works in a conversation on her laptop and wants to follow it from her
phone: she presses the phone mark left of Settings at the sidebar's foot,
*Hand Off to Phone*, confirms it is her with her password or a passkey, and
a QR code appears. Her phone's camera reads it, the phone's browser opens
the conversation she was on, signed in, and the laptop's dialog says
*Handed off to Safari on iPhone.* and closes. On the phone, the page the
code opens says *Handing Off* while it opens the conversation, and *Couldn't
Hand Off* with the reason when the code has expired or was used. The phone mark stands only where the page is not itself on a
phone.

- **Confirming it is the user** takes the password, or a passkey when the
  account has one and the backend offers them, and holds for five minutes
  on that browser, a sign-in counting as one, so a second code in that time asks nothing; a browser
  left signed in cannot hand the account on without its owner.
- **The code** opens the backend's public URL with the page the laptop
  shows, so the phone lands where the user was. It is good for ten minutes
  and once; a new code ends the one before. An expired code's dialog offers
  Show New Code, which asks nothing while the confirmation holds.
- **A phone already signed in keeps its session.** A browser signed in to
  the account opens the page in the session it has, and no second session
  of it is made. A browser not signed in gets a session of its own, as a
  password sign-in on the phone would: signing out on one leaves the other
  signed in. A browser signed in to another account signs that one out and
  takes this one, so it holds one session, never two.

Settings › Account lists the browsers signed in to the account: each by its
browser and system, such as *Safari on iPhone*, or *Web Browser* when its
browser cannot be told, when it signed in and was
last active, the one the user is reading marked *This Browser*. Sign Out
beside another ends that session at once, its open pages included, so a
code photographed off the screen and used by someone else is undone there.

The web app's HTTP requests, the synchronization channel and conversation
WebSockets use the same session cookie.
[Backend authentication](../backend/backend.md#authentication-and-ownership)
owns session and lockout behavior; [Web API](web-api.md#account-api) owns
account request shapes and verification limits. There are no organizations or
team roles.

## Conversations and projects

A conversation owns one agent tree, including its root and subagents, plus
product metadata such as title, target, model selection, archive state, pin and
read state. A project is a named directory on a device; the API calls it a
workspace. It groups conversations without creating a separate execution machine.
The sidebar's Conversations list holds every conversation outside a project,
on the Cloud and on a device alike. One that runs on a paired device shows
that device at its row's very end, after the pin of a pinned row, so the
devices line up down the list, a computer with a dot for the device's state,
as a project's row shows its device, so a row without the mark runs on the
Cloud. For a device since removed, the mark has no dot. A row in a project
shows no mark: its project's row already shows where it runs.

A project's row ends in the same mark, the computer with its state dot, one
component for both rows, so the devices line up down both groups. The mark
stays when the pointer is over the row: the row's buttons, such as a
project's New Conversation and a conversation's Archive, appear to its left,
and the mark's tip names the device and its state, *MacBook Pro · Online*, or
*Removed device*, which its accessible label says too. A project's row names
its device in words only where the mark alone cannot tell two projects apart:
when another project of the same name runs on another device, both rows show
their device's name before the mark, cut at its end when the row is narrow,
and the name gives its place to the buttons while the pointer is over the
row. For example, *demi* on the MacBook and *demi* on the Mac mini read
*demi  MacBook Pro 🖥* and *demi  Mac mini 🖥*, while *chromium*, on the
MacBook alone, reads *chromium 🖥*.

A row's status dot says what the conversation's work is doing, in the colors
an agent's tab and link wear, so one state has one look everywhere:

| State | Dot |
| --- | --- |
| Running | Blue, the accent, breathing |
| Finished, its answer not read yet | Green |
| Failed, not read yet | Red |
| A request waits for the user, not read yet | Yellow, the same size as every other dot and with nothing around it, over every other dot ([Permissions](../agent/permissions.md)) |
| Anything else: read, stopped by the user, or nothing run yet | The faint ring of a settled row |

Every color but the running blue says there is something new to see, so the
row settles to the ring once the user opens the conversation and reads it,
as an unread mark does in Mail: a green answer, a red failure and a yellow
request alike. What happens while the user is reading the conversation is
read as it happens, so its dot does not light. A later event lights the dot
again: a turn that fails after the user read the last one turns it red. A
request read but not answered still waits in the conversation, where its
prompt shows, and in the Running window; only the sidebar settles. An
agent's tab and link have no read state of their own and stay green once
the agent finished. A Stop is the user's own decision, neither finished nor
failed, and leaves the row settled ([Recovering an unfinished turn](#recovering-an-unfinished-turn)).

The sidebar's two groups, Conversations and Projects, each fold under its
label, as Finder's sidebar sections and Slack's do. A chevron after the
label, in the label's faint color, points down while the group is open and
right while it is folded; it shows while the pointer is over the label, and
always on a phone, and a click on the label or the chevron folds or
opens the group. A folded group shows its label alone, with two exceptions
so that nothing the user needs hides in it: the conversation open in the
page stays under the label, with its project's row above it in Projects so
its place still shows, and while a conversation the fold hides waits for
the user and its row is yellow, not yet read, the label carries the yellow dot, between the label and the
chevron. For
example, the user folds Projects to see more conversations; a subagent in a
project asks for a permission, and the dot appears after *PROJECTS* until
the user answers. Which groups are folded is the browser's, as the
sidebar's width is, and stays across reloads.

On hover a row shows Archive at its end, and a pinned row shows Unpin
before it. Pin is in the row's context menu, beside the other changes of a
conversation; a row that is not pinned offers no Pin button on hover, as
Mail and Linear keep the actions they offer on hover few.

```text
User
+-- Cloud device
|   +-- project directory A <- conversations 1, 2
|   +-- project directory B <- conversation 3
+-- connected laptop
|   +-- project directory C <- conversation 4
+-- ungrouped conversations -> Cloud or a direct device directory
```

New conversation opens an immediately typeable draft with Cloud as its default
target. It inherits the user's last explicit model, thinking effort, and service
tier, including a choice made in an empty unsent draft. These defaults are backend
user preferences; existing conversations keep their own selections. If a saved
choice becomes unavailable, the picker keeps it with a warning and requires an
explicit replacement. With no saved choice, the first available model is used.
The sidebar lists a new conversation as *New conversation* from the moment
it opens, whether or not its draft holds anything yet, so the user sees where
they are. Leaving it while its draft is still empty, with no character and no
file, drops it, so opening New conversation and leaving it leaves nothing
behind; once its draft holds something, it stays listed. Until its first
send, such a draft is listed above every conversation with a record, newest
draft first, on every page and after a reload; a conversation created
meanwhile, on this page or another, enters below the drafts. Its first send
gives it the front of the saved order. A drag moves a draft only among the
drafts.

The first send creates the backend record and starts its
[title](#conversation-titles). From then on the backend keeps the
conversation's draft, the message not yet sent: every tab and device that
shows the conversation shows the same draft, a few seconds behind the one
where it is typed, and a send empties it everywhere. When two places edit it
at once, the last save wins, and the composer offers the version it replaced
for a one-click restore. Draft persistence, offline editing and confirmation
of uncertain sends belong to [Drafts](web-application.md#drafts). Choosing a
project or device directory affects subsequent execution; target-switch admission
and context announcements are defined in
[Sessions and targets](../execution/sessions-and-targets.md).

New Project asks where the project lives, with Cloud as the first choice and
Device beside it. The first time, Cloud is chosen. After that the dialog
starts with what the user last chose: the kind, Cloud or Device, and the
device last chosen in its device menu, kept even while Cloud is chosen. A
user who last chose Device and the laptop sees Device and the laptop the
next time; if the laptop has been removed since, Device with no device
chosen. The dialog never goes back to Cloud on its own. Every explicit
choice of a kind or a device saves it, whether or not a project is then
created, as a backend user preference
([User preferences](web-api.md#user-preferences)), so every browser of the
user starts the same way. The directory and the name are not remembered. For
a device, the dialog's button says what it will do with the directory typed:
*Add Project* when it exists, *Create Project* when Demi will create it, as
the directory field's completion has already learned which. The field's
completion list opens with its first entry selected; Up and Down move the
selection, Tab completes the field with the selected entry, and Return
creates the project, as a shell's completion takes Tab and leaves Return to
run. The list closes when the user clicks anywhere outside it, and a click
elsewhere in the dialog never moves the focus back into the field.
Creating the project opens a new conversation in it, as the project's own New
Conversation does: an immediately typeable draft, which the first send makes a
conversation, so a project the user only wanted to set up leaves no empty
conversation behind.

### Where a conversation runs

The conversation's header shows the Host it runs on, the primary Host, with
the number of attached devices beside it (*MacBook Pro +1*). Its menu is one
level, as a Wi-Fi menu lists networks:

```text
Run On
  ☁ Cloud
  🖥• MacBook Pro     This Computer  ✓
  🖥• Studio PC                 LAN
  🖥• Old Laptop             Offline   (cannot be chosen)
  + Add Device…
─────────
Attached
  🖥• build-box       [Detach]
```

- **Run On** lists the Cloud and every paired device with its state; the
  primary Host is checked, and an offline device cannot be chosen. **Add
  Device…** pairs a new one.
- **How this browser reaches each one.** An online device's row ends with
  the path the page uses for it, *This Computer*, *LAN*, *P2P* or *Relay*
  ([Where the P2P path runs](../execution/direct-channel.md#what-the-user-sees)),
  as Wi-Fi menus show each network's security; its latency is the device
  page's.
  The Cloud's row shows nothing, since it is always reached the same way,
  and an offline device's reads *Offline*. The page knows the path of a
  device it has connected to, as it does while it shows a conversation on
  it or its page, and keeps the last it found until the page closes; a
  device it has not tried this time shows no path, since opening a menu
  connects to nothing.
- **Choosing a Host.** In a conversation outside a project, a click moves it
  there at once: to the device's home directory, or the Cloud's directory of
  the conversation. In a project, the Cloud moves it the same way, out of
  its project, and each device but the checked one reads *Studio PC…*:
  choosing it opens that device's directory picker,
  and choosing a directory moves the conversation there, into the project
  whose directory it is, or out of its project otherwise. The header's
  directory menu chooses another directory on the same Host the same way.
- **Attached** lists the devices the conversation's agents attached, by name
  and state. An attached device has no directory or project of its own to
  show: it is a machine the agents may run commands on. The user cannot
  attach one here, since the agents attach what their work needs
  ([What the agent can change](../execution/sessions-and-targets.md#what-the-agent-can-change));
  each row has **Detach**, so the user can always take a device back.

**Telling the agent.** Moving a conversation that has messages asks first,
since the agent's work so far was done in the old place:

```text
  Move this conversation to MacBook Pro?

  It will run in ~/code/shop-app. Files on the Cloud stay there.
  The agent learns of the move when it next works, or now if you tell it.

                    [ Cancel ]  [ Move ]  [ Move and Tell Agent ]
```

*Move and Tell Agent*, the default, moves it and wakes the agent with a
message from the user that names the old and the new place, so it checks what
the move means for its work and says so; the transcript shows it as a receipt
row, *You moved this conversation to MacBook Pro · ~/code/shop-app*. *Move*
moves it only, and the agent reads the move from its execution context at its
next turn ([Switch the primary target](../execution/sessions-and-targets.md#switch-the-primary-target)).
A new conversation that has sent nothing moves without asking.

While the agent works, the header's menus stay choosable, and the same
question says what moving costs, since a conversation moves only while its
work is idle:

```text
  Stop the agent and move this conversation to MacBook Pro?

  The agent and 2 running commands will stop.
  It will run in ~/code/shop-app. Files on the Cloud stay there.

                [ Cancel ]  [ Stop and Move ]  [ Stop, Move and Tell Agent ]
```

Its answers stop the turn, its subagents and its commands as Stop does, then
move the conversation once it is idle, and the second also tells the agent;
a turn that has ended meanwhile moves at once. What will stop is counted from
what the page shows, the agent, its running subagents and commands, as
*The agent, 1 subagent and 2 commands will stop.*, and the move waits for
each of them to end. The offline card's Move to Another Host… asks the same
way. While an archived conversation or a move already under way holds the
menus, their Hosts read as unavailable with *This conversation is archived.*
or *This conversation is moving.* The
directory picker of a project's device asks once a directory is chosen, not
before it opens. Nothing else changes: an offline device still cannot be
chosen.

A conversation enters or leaves a project only through where it runs: its
header's menus, or its agent's commands. The sidebar's menus have no Move To,
since a project is a directory on a Host, and moving a conversation into one
moves where it runs.

Conversations can be archived and restored, or deleted. Archiving is refused
while root or child work or conflicting operations are active. Archiving says
so in a toast with Undo, as Gmail and Linear do. Archived history remains
readable: a conversation in the Archived list opens read-only with a bar that
offers Restore, and sending and metadata changes require restore.

Archiving the conversation the page shows keeps its transcript in view, at
the place the reader is: in the same frame it turns read-only with the bar
that offers Restore, and it never shows a loading state, since nothing in it
changed. Restore makes it writable again the same way, without reading it
anew. For example, the user selects three conversations and archives them,
and one of them has started running in another tab: the backend archives
two and refuses the third, which comes back to the list where it was, and a
toast says why. The archive's toast then reads *2 Conversations Archived*,
and its Undo restores those two.

Delete removes a conversation for good, as deleting a chat in ChatGPT or
Claude does. It is in a conversation's menu in the sidebar, in the menu of a
selection of several, and on each row of the Archived list, and it asks
first: a dialog titled Delete “Fix the login test”?, or Delete 3
Conversations? for a selection, says that the conversation goes with its
messages and the files it holds, that this cannot be undone, and that the
files it changed in its project stay; its Delete button is the destructive
one. A running conversation can be deleted: the deletion stops its work
first, as Stop does, its subagents and commands included. A page that shows
the conversation when it is deleted, the one that deleted it or any other,
goes to a new conversation. A fork of it stays, with its history and its
files. What a deletion removes, and when its stored files go, belongs to
[Deleting a conversation](../backend/storage.md#deleting-a-conversation).

Removing something the user set up, such as a provider, an account of a
subscription, a model added by hand, a subagent profile or a skill source,
asks first in a dialog that names what goes with it, as
revoking a device does; nothing the user set up disappears on one click. Persistent ordering
is independent of activity; its storage rules belong to
[Storage](../backend/storage.md#control-records).

A conversation keeps everything the user wrote and uploaded and everything
its tools produced, images, videos and commands' outputs among them, for as
long as the conversation exists ([Retention](../backend/storage.md#retention)). A
command's output kept on its Host while it runs goes when the conversation's
Host resources are released, once the backend has stored it.

The conversation interface exposes steering, queued messages, stop,
[recovery of an unfinished turn](#recovering-an-unfinished-turn), manual
compaction, model switching, message editing, Fork, and child/terminal
inspection. When an agent's command would act on Demi itself, such as managing the
user's skills, without the user's permission, the command fails, and the
conversation shows the request as a card above its composer and a needs-you
mark in the sidebar until the user allows or denies it for that
conversation; the agent is told the decision
([Conversation permissions](../agent/permissions.md)).
[Message editing](../agent/message-editing.md) and
[Conversation Fork](../agent/conversation-fork.md) define their history
boundaries. Interactive stdin is a `conversation-socket-protocol` capability; exposing a
terminal input control remains separate from read-only job inspection.

### Steer or queue

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

### Saved for later

A message saved for later is one the user keeps to send later, and nothing
ever sends it but the user. It is not the conversation's draft, the message
the composer holds ([Conversations](#conversations)), which it leaves the
composer to become. For example, while the agent works Ana writes *Check prod's
DNS next*, which is not for now: she presses Save for Later beside the send
button, the composer empties, and *1 Saved* appears at the left of the row
under the composer. An hour later she opens it and presses Send on the
message's row.

- **Save for Later** (`NotebookPen`, ⌘S) stands left of Stop and Send while
  the message holds text or attachments, and keeps the whole message, its
  attachments included.
- **The entry** under the composer, at the left of the model's row, reads
  *1 Saved*, *3 Saved*, and is absent while there is none. Pressed, it
  opens the list under the composer, which rises to make room: about four
  rows tall, scrolling within. Opening and closing animate, 150 ms, the
  composer gliding up as the list unrolls beneath it, and not at all with
  reduced motion. The transcript above takes its new height once, at the
  start, and the motion moves only what is drawn, never the layout, so a
  long transcript costs nothing while it plays. Each row shows the message's first line and,
  when it has attachments, their count; pressing the row edits it, and its
  end holds Edit, Send and Delete, always shown, in a quiet color that
  darkens when the row is pointed at, since the row holds nothing else
  there. Where the row under the composer runs
  out of room, the entry shows its mark and the count alone, *7*, with
  *7 Saved* as its tip.
- **Edit** puts the message in the composer, removes it from the list and
  closes the list; a composer holding a message first saves that one for
  later, so nothing is lost. **Send** sends it as the composer would, by the
  same choice of steer or queue while the agent works, and the message shows
  in the transcript, with no toast. **Delete** removes it at once, with Undo
  in a toast. After Send or Delete the list stays open for the next.
- **Saved messages belong to the conversation** and live on the server, so every
  page and device shows the same; archiving the conversation keeps them.

### Recovering an unfinished turn

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
  without either such a record or the user's own Stop.

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
and carries no button.
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
([Waiting for the model](../agent/runtime.md#waiting-for-the-model)), so a
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
([Calls being written](../agent/runtime.md#calls-being-written)), or the
Requesting row. Text the model has finished, with nothing after it yet, is
followed by the Requesting row, its clock counting from the text's end: a
vendor that sends its calls only whole leaves that gap between a sentence
and the call that follows it.

Two other retries are not this and keep their own places: reloading a history
that failed to load, and resending a message whose delivery is unconfirmed.
They recover reading and delivery, not a turn.

### Conversation titles

A user opens a conversation with "why does `pnpm build` fail with TS2307 after
I moved auth into its own package". At once the sidebar shows the start of
that message; a few seconds later it shows "pnpm build TS2307 after auth
package move", written by the model the user picked for the conversation.

```text
first send ──► title = start of the message (80 characters)      origin: message
           └─► one model request, beside the first turn
                    │ a usable line comes back, and the origin is still `message`
                    ▼
               title = the generated line                         origin: generated

rename at any time ──► title = what the user typed               origin: user
```

Later the conversation turns to the login test and the title no longer says
what it is about. The Rename button beside the title lets the user type a new
one where the title stands, which a double-click on the title does too. The
model writes a title once, for the first message; after that the title is the
user's to change.

Every title records its origin, one of `placeholder` ("New conversation",
before any send), `message`, `generated`, and `user`. A `message` title is
the message as the user reads it, plain text: `` `pnpm build` `` gives
`pnpm build`, and no Markdown mark or escape shows, its lines joined by
spaces. A generated title is
written only while the title is still the one the request started from, in the
same statement that checks it, so a rename that lands while the request is in
flight wins and is never overwritten. A rename is a title that differs from
the current one; a patch that repeats the current title, as the web app's
record creation does with the placeholder, changes no origin. A Fork's title,
the source's with " (Fork)", has origin `user`: it is already a settled name
and is not regenerated.

The request:

| Aspect | Rule |
| --- | --- |
| When | At the first send, concurrently with the first turn: it neither waits for the turn nor delays it. A first message without text starts none, and the title stays the placeholder until renamed. There is no other request. |
| Model | The provider and model selected for the conversation at that moment. No separate title model is configured. The messages therefore go only where the user already chose to send them. |
| Effort | The lowest thinking effort the model offers, the default service tier, and an output limit of 4,096 tokens, room for that effort's thinking and one line, or the model's own [output limit](../providers/models.md#output-limit) when it is lower. At 1,024 the baseline model's lowest effort left about one title in ten empty, its thinking spending the whole limit. |
| Input | A fixed title instruction as the system prompt, which says among its rules that a question's title names what it asks, never its answer, and one user message that hands over the user's first message as material to title: *Write the title of the conversation that begins with this message:*, then the message's text as the user reads it, cut to its first 4,000 characters, between `<message>` tags. Given bare, the message is a question the model sometimes answers: *What is 5 times 8?* was titled *40* about once in a hundred. |
| Path | The same metered provider runtime a turn uses, so the request counts in the user's usage ledger and obeys the same limits. It is not a session turn: nothing is added to the transcript and no session event is emitted. |
| Output | The first non-empty line of the text response, without surrounding quotes, cut to 80 characters. Thinking output is ignored. An empty result writes nothing. |
| Failure | A refused, failed or empty request is logged and ends there; the title in place stays, and is already usable. Nothing retries it. |
| Release | The request is aborted when the conversation is archived or the backend closes. |

The limits of 80 and 4,000 characters count Unicode scalar values, and a
cut never splits one.

### Finding a conversation

A user remembers that a conversation explained a TS2307 error, but not which
one. ⌘K, or Search in the sidebar, opens the search window: a field, and
under it the most recent conversations. Typing `TS2307` lists, a moment after
the typing pauses, the conversations whose title or messages contain it, each
with its title, a line of the message around the match, and when the
conversation was last active; the matches are marked in the title and in the
line. The window is as tall as its list, up to a limit past which the list
scrolls, and its field stays where it is as the list grows and shrinks, as
in Spotlight and Raycast. ↑ and ↓ move through the
list; Return or a click opens the selected conversation at the message that
matched, which scrolls into view and is marked for a moment; Escape closes the
window, as ⌘K does in Slack, Linear and ChatGPT. A block brought into view
stands in the middle of the view when it fits there, and from its start, near
the top of the view, when it is taller than the view, so the reader reads it
from its first line, as a browser brings a long section's heading into view.
The mark is the accent's
faint fill behind the row as the row's own hover fill covers it, with its
rounded corners and within the conversation's column, fading out; every
place that marks a block for a moment, a notice's call, a link to a line or
an agent's receipt, marks it the same way.

A query searches what was said: titles, the user's messages, steers among
them, and the text of the answers. Thinking, tool calls, commands' output and attached files are
not searched, since they would bury the message the user remembers under
logs, and neither are subagents' histories, whose conversation its own
messages find. Each word of the query must appear, in any order, ignoring
case. A word matches inside longer text, so `login` finds `relogin`, and a
word of one or two characters, as Chinese often has, matches too.
Conversations whose title matches come first, then those whose messages do,
each group newest first, at most 50 in all. Archived conversations are listed
too, marked Archived, and open read-only as from the Archived list. The
backend answers from an index of each user's conversations
([Search](web-api.md#search), [Search index](../backend/storage.md#search-index)).

### Reading a long conversation

A conversation of a week's work holds thousands of steps, and opening it
takes as long as a short one. For example, the user opens a conversation
whose commands printed megabytes of output: it shows its last answers at
once, as a chat app shows the latest messages first. Scrolling up, earlier
requests appear above as the user nears the top, with a loading row while
they come, and what the user reads never moves. A search result opens the
conversation at its message with the messages around it, and the
scroll-to-bottom button returns to the latest, which kept receiving the
agent's work meanwhile. A folded step shows its title, its outcome and the
files it changed; the first time it is opened, its script and output take a
moment to arrive, shown in place. Switching to another conversation and back
returns to the same place. How the page reads and joins the parts belongs to
[Transcript windows](web-application.md#transcript-windows).

### Following the agent's work

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
  ([Work groups](../agent/runtime.md#work-groups)). A change the backend
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

### Notifications

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
It holds no examples: a model copies an example's language and shape, as one
gave an English request the half-Chinese title "lodash moment request 维护状态"
after the instruction's examples ended in three Chinese ones. It also asks for natural grammar,
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

## Writing a message

The composer shows a message as it will look once sent. For example, a user
types "Compare ", drops `before.png`, types " with ", pastes `after.png`, and
ends with ". The **modal** `padding` is off." The composer reads

```text
Compare [▣ before.png] with [▣ after.png]. The modal padding is off.
```

with "modal" in bold, "padding" set as code, and each file a capsule where it
was put ([Attachments](#attachments)). The conversation shows the sent message
exactly so, the model receives its text and files in that order, and editing
it opens it the same way ([Message editing](../agent/message-editing.md)).

A message is Markdown in the user dialect below. The composer formats a
construct as it is typed, once its closing delimiter is; Backspace right after
gives back the characters. A star typed right before the construct holds it
back, so `**bold**` does not turn italic at its first closing star. A fence
typed as a line of its own opens a code block when the line ends, and typed
as the block's last line closes it. Pasted text is read the same way.

| Written | Shows as |
| --- | --- |
| `**bold**`, `*italic*`, `~~struck~~`, `` `code` `` | Formatted |
| A fenced code block | A highlighted code block |
| A Markdown link, or a bare `http` or `https` URL | A link, its target resolved as in [Files named in messages](file-previews.md#files-named-in-messages) |
| A Markdown image | The image, its target resolved the same way |
| A line break | A line break; a blank line is an empty line |
| `_` and `__`, a single `~`, HTML, `<…>`, math, tables, and lines that start a list, heading, quote, rule or indented code | The characters as typed |

The literal ones are what a conversation about code types as text:
`snake_case`, `__init__`, `~/.zshrc`, `x < y`, `$PATH`, `- item`. Formatting
stays on its line and on its side of a capsule: `**a` and `b**` on two lines
are the characters as typed. Emphasis beside CJK text follows the
[CJK-friendly amendment](https://github.com/tats-u/markdown-cjk-friendly/blob/main/specification.md)
to CommonMark, so `**注意：**这是` is bold as its writer means it. The same
holds wherever Demi reads Markdown: an agent's answer, a Markdown file's
preview, a notification's text and a conversation's title. Plain CommonMark
would show `**建议：**一组` with its stars, since its closing `**` follows a
full-width colon and comes right before a letter, which Chinese, Japanese
and Korean write without a space.

The model receives the message's Markdown. Where the user typed literally a
character the dialect would read as formatting, it carries a backslash
(`\*args`); nothing else is escaped, so `snake_case` and `x < y` reach the
model as typed.

Copying and pasting never add line breaks of their own: a copy of a
message, whole or a selection of it, holds no blank line before its first
line or after its last, and a paste into a composer drops the blank lines at
the start and end of what it pastes, keeping the lines between them and each
line's indentation. For example, the user copies a sent message and pastes it
back into the empty composer: the composer holds exactly the message, with no
line under it.

A selection that runs past one message, from the user's message into the
answer, copies as a web page does: the text of everything selected, in
order, as it reads on screen, with none of a message's own forms. For
example, the user drags from their question into the answer and copies: the
clipboard holds both, the question as shown rather than its Markdown.

Copying a sent message, by its Copy control or by selecting part of it,
copies its files with its text, and pasting that into a composer brings the
files back as capsules, where they stood, without uploading them again. For
example, the user copies "Compare `before.png` with `after.png`" from a sent
bubble and pastes it into the composer of another conversation of theirs:
the composer holds the same text and two image capsules, ready to send. The
clipboard carries the copy in three forms, each for the place it is pasted:

| Form | What it holds | Who reads it |
| --- | --- | --- |
| Demi's own | The message's content: its text, and each file as the reference the message holds (its stored file, name, kind and size, or its device and path) | A composer of the same Demi; it rebuilds each capsule from its reference, so the file is not sent again |
| HTML | The text formatted, each image as an image | Mail, documents, chat apps |
| Plain text | The Markdown, each file by its name | Terminals, code editors, anything else |

Demi's own form travels inside the HTML, as an attribute of its outermost
element, since every browser keeps HTML on the clipboard and none keeps a
format of its own across pages; a paste reads it there first. A copy that
holds a single image and no text also carries the image itself, so pasting
it anywhere pastes the picture. A file whose reference the paste cannot use,
one of another account or since deleted, comes in from the image the copy
carried when there is one, uploaded as any pasted image is, and otherwise
as its name in the text, with a toast that says which file could not be
added. Copying never puts a file's bytes on the clipboard for the composer's
sake; the reference is the file. A paste lands at once, its text and its
capsules, and each capsule attaches its stored file beside it as an upload
does, showing that it is under way, so the message can be sent at once and
waits for them as for an upload. A picture pasted in place of a reference
keeps the copied file's name.

Enter sends and Shift+Enter breaks the line; in a code block Enter breaks the
line and ⌘/Ctrl+Enter sends. While the agent works, ⌘/Ctrl+Enter outside a
code block sends the other way than Enter
([Steer or queue](#steer-or-queue)); in a code block it is the send key and
sends the way Enter does elsewhere. An input method's Enter never sends, and
its Escape never ends an edit ([Typing with an input method](#typing-with-an-input-method)). A
message whose files are still uploading is sent like any other: it leaves
the composer, shows at once as the user's bubble with each capsule showing
how far its upload has come, and the tail row says Requesting from that
moment; it reaches the agent once every upload has finished, and never
without one of its files. For example, the user drops a 40 MB recording,
types "what goes wrong here?" and presses Enter: the bubble shows the
recording's capsule at 30%, and the turn starts when it reaches 100%. An
upload that fails leaves the message undelivered, its bubble saying so with
Retry, which retries the upload and then delivers the message; no file of it
is dropped. A page closed while such a message waits asks first, as a mail
client does while it sends attachments. Enter on a message that cannot go
yet for another reason does not send in silence: the send button says why it
cannot, in the words it says when it is pointed at — an upload that failed,
a model that cannot send — and the answer goes by itself. A message sent or queued, by Enter or by the send button, leaves the
focus in the composer, so the next one is typed at once. An edit sent with
Save and resend takes no typing while it is saved; once it ends, the
composer's draft takes the focus, unless the user has put it elsewhere
meanwhile. The composer and the conversation render a user message with one
editor, read-only in the conversation, so a sent message cannot look
different from what was written.

The composer stands one line high while the message fits that line. As soon
as the message needs more — a line break, a code block, an image, or text
wider than the line — the composer opens into a box that wraps the text,
grows to six lines and scrolls beyond them; it closes again once the message
fits one line. No part of a message is ever cut off at the line's end.

### Typing with an input method

For example, the user types `nihao` with a Chinese input method in the
filter of a menu and presses Enter to confirm 你好: the filter shows 你好,
and no row is picked. A second Enter picks the highlighted row.

A key that an input method uses to confirm or cancel a candidate, such as
Enter, Escape, Tab or an arrow, belongs to the composition. In every text
field of the app, the composer, a rename, a search, a dialog's field or a
menu's filter, it sends, submits, picks, confirms or closes nothing. This
holds in every browser, Safari included, which delivers the confirming Enter
just after the composition has ended, as Chrome and Firefox do not. The
`web-ui` text fields, the composer's editor and every key handler on them
pass such a key through one shared guard, as VS Code, Lexical and Headless
UI each do in one place.

## Attachments

Short pasted text stays in the composer and sends as message text. A paste of at
least 2,000 characters or 40 lines becomes `pasted-text.txt`, using the shared
composer's paste thresholds. Dropped, selected, and pasted files use the same
staged attachment flow, and editing a sent message adds files the same way.

The composer holds two things and nothing else: text in the user dialect,
and capsules. Whatever enters it, typed, pasted or dropped, from Finder, a web
page, another app or Demi's own transcript, is turned into those as it
enters, never kept as what it was. For example, the user drags a screenshot
from the agent's reply, or an image from a web page, onto the composer: it
lands as an `image.png` capsule where it was dropped, exactly as a pasted
screenshot does, never as a picture set into the text. So:

- A file, or an image whose bytes the page can read (dragged or pasted as a
  file, a `data:` image, an image of Demi's own page), becomes a capsule; an
  attachment of the conversation dragged from the transcript becomes a
  capsule of the same file, uploading nothing.
- An image the page cannot read, such as one another site serves without
  letting the page fetch it, becomes its link as text.
- Pasted or dropped rich text keeps only the dialect's formats; images in it
  follow the two rules above, and every other element, a table, a layout,
  a style, becomes its text.
- A capsule dragged within the composer moves; one dragged from another
  composer, as from a message being edited, is copied as a capsule.

Each attachment is a capsule in the message's text, and that text is the
editor's document. The document is the message: it says which files the
message carries, in what order, and what each one is. Nothing outside it adds
a file to a message or takes one away.

A capsule carries what it shows and what the message sends: the file's name
and kind, its picture when it has one, the device and path of a file that
lives on another device, and the opening lines of a text file. An uploaded
file's media type and opening lines come from the backend, in the answer to
its upload ([Uploads and media](web-api.md#uploads-and-media)), so the capsule
shows the file as the backend will send it. Bytes on their way to the Host
cannot be in a document, so they wait beside it under the capsule's id: the
file itself, how far its upload has come, whether it failed. That is a
transfer, not a fact of the message, and a capsule with no transfer beside it
is a file that needs none — a restored draft, or a sent message.

What follows from that split:

- A capsule is a character of the message and is edited as one. Backspace
  deletes it, undo brings it back, and it can be dragged to another place in
  the text; it carries no control of its own for any of that.
- A dropped file lands where it is dropped, a picked or pasted one at the
  cursor, and one added from a file browser at the cursor. A message holds any
  files the user adds, two of the same name included, as two screenshots
  pasted one after the other are both `image.png`: a name says nothing about
  a file's contents, and the Host keeps both, under its own suffixes
  ([Sending a file](#attachments)). Only an empty file and one over 25 MB are
  refused. A file joins the
  message as one change to its document, so there is never a file without a
  capsule or a capsule without a file.
- Deleting a capsule takes its file out of the message and stops its upload.
  The transfer waits, so undo brings the file back with the capsule and the
  upload starts over. What is still waiting goes when the message is sent or
  the composer is left.
- While a file uploads its capsule shows how far along it is, and a failed
  upload offers Retry. Pointed at, a capsule shows the picture larger or a
  text file's opening lines. A sent message shows every file as the
  composer did, an image or a video included: a capsule in the line, a
  small gap from the text and the capsules beside it. In the conversation,
  a click on a capsule opens its file in the File view.
- The message's content is read off the document: for the example in
  [Writing a message](#writing-a-message), `Compare `, then `before.png`,
  then ` with `, then `after.png`, then the rest. Providers pass content in
  order, so the model meets each file where the user put it.
- The saved draft keeps the message's text with a mark where each capsule
  stands and, beside it, each file as its upload or its device and path, so
  opening the draft on any page builds the same document again.

Sending a file proceeds through three owners:

```text
Composer file -> backend upload/blob -> selected Host attachment directory
                         |                          |
                         +-- attachment ID          +-- absolute file path
                                                       in the agent message
```

Files remain staged in the draft until send, each as its upload, so another
device shows the file and can send it. The backend writes the file under the Host
user's `~/.demi/attachments/<conversation>/`, adding a numeric suffix to avoid
an existing name of other contents; a file of the same name and the same
contents already there is the same file, and is not written again. The suffix
belongs to the Host path alone: the attachment keeps the name the user gave
it, so a file sent again, pasted or put back by Recall keeps its name,
`login-before.png`, and never grows `-2-2`. It does not put attachment files in the project directory.
A Cloud target may need to wake before this write.

The agent message contains one `attachment` record with name, path, media type,
size, and blob hash; a text file's record also carries its opening lines, which
the capsule shows when pointed at. The model receives the record as a tag
that names the file ([Replay](../agent/runtime.md#replay)), so it can read the
file with tools. The complete text
file is not duplicated into the message.

Native media adds the corresponding image, video, audio, or document input beside
the attachment record. An image enters fitted to what every provider accepts,
while the attachment keeps the original
([Images in the transcript](../agent/runtime.md#images-in-the-transcript)). The
attachment remains one capsule. The accepted extensions of each request's model
govern what the request carries: a medium the model does not accept, or one
too large for its requests, travels as a text that names it and says why
([Replay](../agent/runtime.md#replay)), and an adapter must not replace
supported media with a placeholder. Other files remain
accessible by path. Attachment presence and model capability are separate facts.

The upload cap is 25 MiB. Upload IDs and transcript media references travel in
conversation frames; bulk bytes use HTTP. Exact wire forms, ownership checks,
missing-upload behavior, and blob delivery to the web app are defined in
[Backend media handling](../backend/backend.md#media-by-reference) and
[Web API uploads](web-api.md#uploads-and-media).

A remote-file selection is different from an upload: it names an existing file
on a connected device, and its capsule names the device. Its bytes are read
when the model executes the supplied host command, so it is not a snapshot.
Revocation, disconnect, or file changes can affect that later read.

## Provider management

Models & providers is available to users permitted to configure their provider
scope. The model picker groups models by entry. For a model whose window is
over 500,000 tokens, the model menu also offers a Context row: a smaller
window that every conversation of the user with that model then uses
([Context limit](../providers/models.md#context-limit)). Adding an entry has three paths:

| Path | User input and result |
|---|---|
| Subscription | Claude sign-in, or Codex/Grok device login; additional accounts go into the existing family entry |
| Vendor catalog | Select vendor, label, key, and optional endpoint/model overrides; the family is derived from supported vendor metadata |
| Custom endpoint | Select family and protocol, then label, endpoint, key, and model configuration |

Entries can be renamed, edited, or removed. API entries support a manual model
list; clearing it returns to catalog discovery. Labels need not be unique.
Provider configuration stores explicit endpoint and model overrides. Fetched
model directories are cached; they are not a fresh network lookup on every
conversation opening.

The page displays authentication/runtime state, account selection, and available
quota information. Reading status must not run inference. An explicit Test
action can make a real request; a provider whose transport is a process runs
it on the acting user's Cloud, which wakes for it
([Where it runs](../providers/claude-code.md#where-it-runs)). Hiding a
provider or model from the picker is a web app preference and does not stop
existing work. Provider protocol and credential details have their
authoritative home in [Providers](../providers/providers.md), catalogs in
[Models](../providers/models.md), and quota in
[Usage and quota](../providers/usage-and-quota.md).

## Web app scope

The web app provides conversations, account settings, provider management,
devices, and Cloud controls. Administrative account management and usage have
backend APIs; their dedicated pages in the web app are deferred. Settings
list the backend's plugins, each with a switch that turns it on or off for
the user, and an open conversation whose commands a switch changed offers a
reload ([A user's plugins](../architecture/plugins.md#a-users-plugins)).
Settings have a Subagents section ([Profiles](../agent/subagents.md#profiles)):
a switch that turns subagents on or off for the user, on for a new user; the
two [limits](../agent/subagents.md#limits), Maximum subagents and Maximum
depth, each a number with Unlimited as a choice, which asks first in a
dialog warning that it can make conversations unstable, the dialog's
default button keeping the limit, and which keeps the number, so choosing
Limited again shows the limit as it was; and the user's subagent profiles, which the user creates, edits and deletes
there, each with a switch of its own that enables or disables it; a new
profile starts enabled. The next spawn of every conversation uses a change.
A profile's model is chosen with the composer's own model menu, the same
control with the same items, providers, models, Reasoning and the rest, plus
*Parent's model* at its top. A profile whose provider entry, model, effort
or tier is gone is marked as unavailable, with the part that is missing,
until the user fixes it.
Settings have an Instructions section, after Providers: one text
field for the user's [personal instructions](../agent/instructions.md#personal-instructions),
which every conversation's agents follow, with Save, available while the
text differs from the saved one, and the count of characters against the
limit. Saving an empty text removes them. The context card's Personal
instructions entry opens this section.
Skills have a settings section of their own, which the sidebar also opens
([Skills](../agent/skills.md#the-page)).
Data and privacy actions, language switching, and account deletion are also
deferred. Exporting conversations and public share links are not offered, and
nothing in the web app leads to them.

The selected scope excludes public sharing, collaboration, offline mode,
PWA behavior, push notifications, and localization. Technology and package
responsibilities belong to [Web architecture](web-application.md). Components,
layout, and interaction examples are maintained in the gallery.

## Cloud settings

Each user has one Cloud device shared by all their Cloud projects. Those projects
can access each other's files; they are not isolation boundaries. Creating or
removing project metadata does not allocate or delete a machine.

Settings shows the Cloud's storage and Reset environment, but not its lifecycle
state. A stopped Cloud is not unavailable: the next operation that needs it
wakes it ([Resolve a target](../execution/sessions-and-targets.md#resolve-a-target)),
and a label such as "Sleeping" would suggest otherwise. Storage is the current
capacity of each writable filesystem, system and home, against the most it may
grow to ([Lifecycle and capacity](../cloud/managed-hosts.md#lifecycle-and-capacity));
while the capacities are unknown, as before the Cloud's first start, it shows
only the limits.

When the server has moved to a release with a newer Cloud image than the one
the Cloud's system is on, Settings says so beside Reset environment: "A newer
system image is available. Reset the environment to use it." The Cloud keeps
working on its image meanwhile, with the new release's Demi programs
([Demi's programs in a Cloud](../cloud/managed-hosts.md#demis-programs-in-a-cloud)),
and the user resets when losing the system's changes suits them.

The confirmation explains that reset stops all of the user's Cloud work, replaces
system packages and settings, and preserves home files. Acceptance starts an
operation; the confirmation follows its progress and offers retry of the same
operation ID on failure, including when the guest is offline. Project links and
history remain intact. Reset is a user Cloud action, not a project action.

[Managed hosts](../cloud/managed-hosts.md) owns lifecycle and durability
guarantees. `web-ui` owns the Cloud settings and the reset interaction, while
product and gallery provide real state or fixtures to that same component.
