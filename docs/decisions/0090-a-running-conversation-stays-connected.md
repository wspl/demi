---
status: proposed
---

# A running conversation stays connected

Supersedes: 0025 (A transcript is shown from a bounded window, around any
message, local copy first), the part that lets a conversation go once it is
not among the few shown most recently.

## Problem

Alex works in five conversations at once, each with an agent building
something. They show *Fix the login page*, then four others, then go back to
*Fix the login page*. Its agent kept working the whole time, but the page let
the conversation go after the third switch: it closed its connection and
stopped writing its copy on disk. Going back shows that copy, which is the
state of four switches ago, and a moment later the turn that ran meanwhile
appears all at once. Every switch back to a working conversation shows old
data first, then a jump.

A conversation whose turn starts while it is let go, because a schedule fired
or another conversation sent it a message, does the same when Alex opens it.

## Decision

A conversation stays connected while its turn runs, however long ago it was
shown and however many were shown since. Going back to it shows what it holds
now, with no jump.

* **Eight conversations shown most recently stay open**, up from three, as
  people who work in several conversations switch between more than three.
* **A conversation that runs stays open** beyond those eight. Its status in
  the list is *running* or *compacting*. It is let go only once its turn ends
  and it is not among the eight, when its copy on disk holds the end of the
  turn, so opening it later shows the turn whole.
* **A conversation that starts to run opens**, in the background, though the
  page holds nothing of it: the list channel tells every page each change of
  a conversation's status, a schedule's run or a message from another
  conversation included. It opens as a shown one does, its copy on disk
  first, then its socket, but nothing on screen changes.
* **A conversation stays open until the page has caught up with it**: its
  opening has ended, its socket is connected, and the root's transcript is
  current. Until then the page cannot tell that its turn ended, and letting
  it go would leave its copy without the end of the turn.
* **Opening in the background shows nothing**: a message still on its way
  that the opening sends again does not make the conversation the one the
  page shows, so the page never tells the backend that it shows a
  conversation it does not, nor marks one read.
* A conversation with a message still on its way stays open, as today.

```
  shown recently           running             open?
  (most recent first)
  1 .. 8                   any                 yes
  9 ..                     yes                 yes, until its turn ends
  9 .. or never shown      no                  no
```

The rule lives in `packages/web/src/conversation/store.ts`, which owns the
conversations the page holds:

```ts
/** How many conversations shown most recently stay open, the one shown among them. */
const KEPT_CONVERSATIONS = 8

/** Whether the page holds the conversation but not yet what its session says now. */
function catchingUp(conversation: Conversation): boolean

/** Whether the page keeps the conversation open: shown recently, running, catching up, or sending. */
function kept(conversation: Conversation): boolean

/** Opens a conversation the page does not show, as a shown one opens. */
function openInBackground(conversation: Conversation): Promise<void>
```

`letGoBeyondKept` lets go each conversation the page holds that `kept` no
longer answers for, including one it opened in the background and never
showed. It runs when the shown conversation changes, when a conversation's
status leaves *running* or *compacting*, and when one catches up.
`catchingUp` reads a new `ConversationRuntime.caughtUp`, from the client's
new `holdsCurrent(node)`. A delivery to a conversation the page does not
show opens it with `openInBackground` instead of showing it. A status that becomes
*running* or *compacting* for a conversation the page does not hold calls
`openInBackground`, and so does each running conversation the list names when
the page loads.

Nothing is stored and no protocol changes: the backend already sends each
status change on the list channel and already serves several sockets of one
page.

## Spike

None: the list channel already carries each status change, which
`status_changed` in `backend-user-shard` marks, and a held conversation's
memory is bounded by the transcript window 0025 introduced.

## Prior art

* [Open WebUI](https://github.com/open-webui/open-webui) keeps one socket
  for the page and joins only the shown chat's room
  (`src/lib/components/chat/Chat.svelte`, `syncChatRoom`); events of other
  chats are ignored, and going back to one reads it again from the server.
* [LibreChat](https://github.com/danny-avila/LibreChat) closes a chat's
  stream on navigation, while the generation goes on, and resumes it when the
  chat is shown again (`client/src/hooks/SSE/useResumeOnLoad.ts`), replaying
  what it missed.
* [matrix-js-sdk](https://github.com/matrix-org/matrix-js-sdk), which
  Element is built on, keeps every joined room's live timeline current from
  one sync (`src/sync.ts`), shown or not.

Demi follows Matrix's clients for the conversations that change: Open
WebUI's and LibreChat's read on return is the jump this record removes. It
departs from them in keeping only the running ones and the eight shown most
recently, because a Demi conversation holds far more than a chat room's
latest messages.

## Alternatives

* **Keep every conversation open.** Memory and sockets would grow with every
  conversation the list holds, most of which never change while the page is
  open.
* **Keep more shown conversations and nothing else.** A working
  conversation beyond the count still jumps, and one a schedule started
  still opens from an old copy.
* **Follow every conversation's changes on the list channel.** The list
  channel would carry every transcript, which is the conversation socket's
  job.

## Open questions

None.
