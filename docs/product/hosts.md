# Hosts

How the product shows the computers a conversation runs on: a device's mark, the header's host menu and directory, and the Cloud's settings.

## A device's mark

**A device's mark** is the same wherever a device is drawn: the sidebar, the
host menu, Settings' devices and the file browser. It is the logo of the
device's system, so the user tells the computers apart at a glance without
reading a name:

```text
 ●   the state dot, top right (none for a removed device)
 ▉   the system's logo: Apple for macOS, the Windows logo, for Linux the
 2   distribution's logo (Ubuntu, Debian, Fedora, Arch Linux, CentOS,
     Red Hat, Linux Mint) or Tux for any other; the number, bottom right
```

- **The number** tells apart devices whose logos are the same: with two Macs
  they read Apple 1 and Apple 2, while the user's only Windows PC and an
  Ubuntu server show no number. A device's number is given once, when its
  runner first reports its system, as one more than the highest number among
  the user's devices with the same logo, and it never changes while the
  device stays, so a number the user learned keeps meaning the same
  computer; if a reinstall changes its system, it takes a new number for its
  new logo. The number shows only while another of the user's devices has
  the same logo, so pairing a second Mac adds a *1* to the first.
- **How it is drawn:** the logo in the subtle icon color, about the size of
  the computer glyph it replaces; the state dot a size smaller than the one
  the computer glyph wore, at the logo's top right edge; the
  number is a small bold digit cut out of the logo by an outline in the
  color behind it, as a new file's plus is (`CornerPlus`), with no round
  background, its center in line with the dot's above it. A two-digit number
  is drawn the same way and may reach further left.
- **Before its runner first connects** a device's system is known only by its
  platform: a Mac shows the Apple logo and a Windows PC the Windows logo,
  while a Linux device shows Tux until its runner names its distribution.
- **The Cloud** keeps the cloud glyph and never shows a number, and a device
  since removed shows the plain computer glyph, without a dot.
- The tip and the accessible label are unchanged, *MacBook Pro · Online*:
  the logo and the number only help the eye.

## Where a conversation runs

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
  it or its page. A device it has not connected to yet shows the path this
  browser found last time, in a fainter color than a current one, since
  opening a menu connects to nothing and the network may have changed
  since; the browser keeps each device's last path, and a device it never
  reached shows none. For example, the Windows PC reached by relay
  yesterday reads *Relay*, faint, until the page next connects to it.
- **Choosing a Host.** In a conversation outside a project, a click moves it
  there at once: to the device's home directory, or the Cloud's directory of
  the conversation. In a project, the Cloud moves it the same way, out of
  its project, and each device but the checked one reads *Studio PC…*:
  choosing it opens that device's directory picker,
  and choosing a directory moves the conversation there, into that
  directory's project, which the move makes when there is none, or out of
  its project when the directory is the device's home. The header's
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

  It will run in ~/code/website. Files on the Cloud stay there.
  The agent learns of the move when it next works, or now if you tell it.

                    [ Cancel ]  [ Move ]  [ Move and Tell Agent ]
```

*Move and Tell Agent*, the default, moves it and wakes the agent with a
message from the user that names the old and the new place, so it checks what
the move means for its work and says so; the transcript shows it as a receipt
row, *You moved this conversation to MacBook Pro · ~/code/website*. *Move*
moves it only, and the agent reads the move from its execution context at its
next turn ([Switch the primary target](../execution/sessions-and-targets.md#switch-the-primary-target)).
A new conversation that has sent nothing moves without asking.

While the agent works, the header's menus stay choosable, and the same
question says what moving costs, since a conversation moves only while its
work is idle:

```text
  Stop the agent and move this conversation to MacBook Pro?

  The agent and 2 running commands will stop.
  It will run in ~/code/website. Files on the Cloud stay there.

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
[recovery of an unfinished turn](turns.md#recovering-an-unfinished-turn), manual
compaction, model switching, message editing, Fork, and child/terminal
inspection. When an agent's command would act on Demi itself, such as managing the
user's skills, without the user's permission, the command waits, and the
conversation shows the request as a card above its composer and a needs-you
mark in the sidebar until the user allows or denies it for that
conversation; the agent is told the decision
([Conversation permissions](../agent/permissions.md)).
[Message editing](../agent/message-editing.md) and
[Conversation Fork](../agent/conversation-fork.md) define their history
boundaries. Interactive stdin is a `conversation-socket-protocol` capability; exposing a
terminal input control remains separate from read-only job inspection.

## Cloud settings

Each user has one Cloud device shared by all their Cloud projects. Those projects
can access each other's files; they are not isolation boundaries. Creating or
removing project metadata does not allocate or delete a machine.

Settings shows the Cloud's storage and Reset environment, but not its lifecycle
state. A stopped Cloud is not unavailable: the next operation that needs it
wakes it ([Resolve a target](../execution/sessions-and-targets.md#resolve-a-target)),
and a label such as "Sleeping" would suggest otherwise. Storage is the
capacity of each writable filesystem, system and home
([Lifecycle and capacity](../cloud/managed-hosts.md#lifecycle-and-capacity)),
such as *System 16 GB · Home 16 GB*; while the capacities are unknown, as
before the Cloud's first start, the row is not shown.

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
