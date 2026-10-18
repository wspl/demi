# Conversations and projects

What a conversation is, how projects group conversations by the directory they run in, and how the user finds, names, saves and reads them. Where a conversation runs is in [Hosts](hosts.md), and what happens while its agent works in [Turns](turns.md).

## Conversations and projects


A conversation owns one agent tree, including its root and subagents, plus
product metadata such as title, target, model selection, archive state, pin and
read state. A project is a named directory on a device; the API calls it a
workspace. It groups conversations without creating a separate execution machine.

**Where a conversation runs decides its project, and nothing else does.** A
conversation that runs in its Host's home directory, or on the Cloud in a
conversation's directory (`/home/demi/sessions/<id>`), its own or, after a
Fork, its source's, is outside any project; one that runs in any other
directory is in that directory's project. A home is never a project: New
Project refuses a device's home, *Your home folder holds the conversations
outside projects. Choose a folder in it.* When no project
has that directory yet, moving the conversation there makes one, named after
the directory's last component, with *(2)*, *(3)* after it when the device
already has a project of that name. For example, an agent that runs `demi host
move work-mac /Users/alex/projects` puts its conversation in a new project
*projects* on work-mac, and moving it to `/Users/alex` puts it back among
Conversations. A directory is its own project: `~/code/demi/packages` is not
the project of `~/code/demi`, but a project of its own once a conversation
runs there.

The sidebar's Conversations list holds every conversation outside a project,
on the Cloud and on a device alike. One that runs on a paired device shows
that device at its row's very end, after the pin of a pinned row, so the
devices line up down the list, the device's mark (below), as a project's row
shows its device, so a row without the mark runs on the Cloud. A row in a
project shows no mark: its project's row already shows where it runs.

A project's row ends in the same mark, one component for both rows, so the
devices line up down both groups. The mark
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
failed, and leaves the row settled ([Recovering an unfinished turn](turns.md#recovering-an-unfinished-turn)).

The browser tab carries the same news, so a user working in another tab
sees that something waits, as Gmail's and Slack's tabs show it. The page's
title is the open conversation's title and the product's name,
*整理季度报告 · demi*, or *demi* alone where no conversation is open.
While any conversation's dot is green, red or yellow, the title starts with
one count: of the one kind there is, *(2 done) 整理季度报告 · demi*,
*(1 failed)*, *(1 to do)* for yellow, what waits for the user; and when
there are several kinds, all of them as to do, *(4 to do)*, since one short
count reads at a glance where a list does not. In Chinese *(2 个完成)*,
*(1 个出错)*, *(1 个待办)* and *(4 个待办)*.
The tab's icon wears one small dot in its lower right corner, touching its
edges, in the most urgent of those
colors, red over yellow over green, with a ring of the tab strip's
background around it, and none while nothing is new. The conversation open
in a focused page is read as it happens and counts for nothing. An installed
demi, on a Home Screen or in the Dock, shows the number of those
conversations on its icon, as Mail counts unread mail, through the
browser's app badge, and clears it at zero. All of it follows the sidebar's
dots as they change, from the same state, never from a timer.

They are a reminder to come back, so coming back clears them: when the
user brings a page forward, its title count, its icon's dot and the app's
badge go, and only what lights after that counts again, as a phone's
notification badge clears once the app opens. The sidebar's dots stay
until each conversation is opened, since they say which ones are still
unread.

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

On the desktop, on hover, a row shows Archive at its end, and a pinned row shows Unpin
before it. Pin is in the row's context menu, beside the other changes of a
conversation; a row that is not pinned offers no Pin button on hover, as
Mail and Linear keep the actions they offer on hover few. On a phone the
row shows neither, and its long press opens its menu
([The phone layout](web-application.md#the-phone-layout)).

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
([User preferences](web-api-settings.md#user-preferences)), so every browser of the
user starts the same way. The directory and the name are not remembered. For
a device, the dialog's button says what it will do with the directory typed:
*Add Project* when it exists, *Create Project* when Demi will create it, as
the directory field's completion has already learned which. The field's
completion list opens with its first entry selected; Up and Down move the
selection, Tab completes the field with the selected entry, and Return
creates the project, as a shell's completion takes Tab and leaves Return to
run. The list closes when the user clicks anywhere outside it, and a click
elsewhere in the dialog never moves the focus back into the field.
A directory that is already a project opens that project instead of
making a second, as choosing an existing folder in Finder's sidebar opens
it. Creating the project opens a new conversation in it, as the project's own New
Conversation does: an immediately typeable draft, which the first send makes a
conversation, so a project the user only wanted to set up leaves no empty
conversation behind.

**Deleting a project** archives its conversations, so a project never comes
back by itself from a conversation still running in its directory. Delete
Project… in the project's menu first asks, naming what goes: *Delete
“website”? Its 3 conversations will be archived. Files on MacBook Pro are not
deleted.*, with Cancel and Delete. A project with no conversation asks too,
without the second sentence. Restoring one of those conversations later
makes the project again, as a move there would. A conversation of the
project that is running refuses the deletion, as it refuses its archive.

## A conversation's workflow

The composer's lower left has the Workflow menu, which shows its choice,
such as *Default*. Default follows Settings › Subagents, No Subagents turns
subagents off for this conversation, and each workflow the project keeps in
`.demi/workflows/`, a group of subagent profiles with instructions on how to
use them, is an option of its own, by its name
([Workflows of a project](../agent/subagents.md#workflows-of-a-project)).
The choice is the conversation's, kept with it as its model is, and applies
from the next spawn; subagents already running go on as they were started.

## Saved for later

A message saved for later is one the user keeps to send later, and nothing
ever sends it but the user. It is not the conversation's draft, the message
the composer holds ([Drafts](web-application.md#drafts)), which it leaves the
composer to become. For example, while the agent works Ana writes *Check the
server's DNS next*, which is not for now: she presses Save for Later beside the send
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
- **Nothing is lost on a refusal.** A save the server refuses, as when the
  conversation already keeps 200, leaves its row in the list as *Not
  Saved* in secondary red text, with Retry and Delete beside Edit; an edit
  refused leaves the message in the list and what the composer held in a
  *Not Saved* row the same way, so neither text is only on the page's
  screen a moment and then gone. A Not Saved row is this browser's, kept
  across a reload, since the server refused it; the entry's count includes
  it. A text in such a row is not also left in the composer.
- **Saved messages belong to the conversation** and live on the server, so every
  page and device shows the same; archiving the conversation keeps them.

## Sharing

The header's **⋯** menu shares the conversation, and an answer's **Share**,
beside Fork, shares that answer, each as a read-only snapshot anyone with
its link can read ([Sharing](sharing.md)).

## Conversation titles

A user opens a conversation with "why does `pnpm build` fail with TS2307 after
I moved auth into its own package". At once the sidebar shows the start of
that message; a few seconds later it shows "pnpm build TS2307 after auth
package move", written by the user's small model.

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

Every title records its origin, one of `placeholder` (before any send),
`message`, `generated`, and `user`. A `placeholder` title holds no words:
the page shows it as *New conversation* in the reader's language
([Localization](localization.md)), so a conversation started in one
language reads right in another. A `message` title is
the message as the user reads it, plain text: `` `pnpm build` `` gives
`pnpm build`, and no Markdown mark or escape shows, its lines joined by
spaces. A generated title is
written only while the title is still the one the request started from, in the
same statement that checks it, so a rename that lands while the request is in
flight wins and is never overwritten. A rename is a title that differs from
the current one; a patch that repeats the current title, changes no origin. A Fork's title,
the source's with " (Fork)", has origin `user`: it is already a settled name
and is not regenerated. The Fork of a conversation still untitled is
untitled too, origin `placeholder`.

The request:

| Aspect | Rule |
| --- | --- |
| When | At the first send, concurrently with the first turn: it neither waits for the turn nor delays it. A first message without text starts none, and the title stays the placeholder until renamed. There is no other request. |
| Model | The user's [small model](../providers/models.md#small-model), resolved for the conversation at that moment: a fast model with thinking off, chosen in Settings › Agent; the conversation's own model when the user has no small model. |
| Effort | The small model's thinking, the default service tier, and an output limit of 4,096 tokens, room for the lowest effort's thinking and one line, or the model's own [output limit](../providers/models.md#output-limit) when it is lower. At 1,024 the baseline model's lowest effort left about one title in ten empty, its thinking spending the whole limit. |
| Input | A fixed title instruction as the system prompt, which says among its rules that a question's title names what it asks, never its answer, and one user message that hands over the user's first message as material to title: *Write the title of the conversation that begins with this message:*, then the message's text as the user reads it, cut to its first 4,000 characters, between `<message>` tags. Given bare, the message is a question the model sometimes answers: *What is 5 times 8?* was titled *40* about once in a hundred. |
| Path | A [small request](../providers/models.md#small-model), purpose `title`. It is not a session turn: nothing is added to the transcript and no session event is emitted. |
| Output | The first non-empty line of the text response, without surrounding quotes, cut to 80 characters. Thinking output is ignored. An empty result writes nothing. |
| Failure | A refused, failed or empty request is logged and ends there; the title in place stays, and is already usable. Nothing retries it. |
| Release | The request is aborted when the conversation is archived or the backend closes. |

The limits of 80 and 4,000 characters count Unicode scalar values, and a
cut never splits one.

## Finding a conversation

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
([Search](web-api-conversations.md#search), [Search index](../backend/storage.md#search-index)).

## Reading a long conversation

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
