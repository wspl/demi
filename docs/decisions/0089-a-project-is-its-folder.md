---
status: proposed
---

# A project is its folder: named by it, kept when deleted, and sharing its saved messages

## Problem

Alex keeps the messages they send again and again in the composer's Saved
list: "Run the tests and fix what fails", "Rebase on main and push". The
list belongs to one conversation. Each new conversation of the
`checkout-web` project starts with an empty list, so Alex saves the same
messages again in every conversation, or copies them from an old one.

Two more problems come from treating a project as a record of its own
rather than as the folder it is:

* Deleting `checkout-web` deletes its record. Restoring one of its archived
  conversations makes a new project for the folder, which starts over: the
  model and workflow Alex chose there are gone.
* A project's name is stored apart from its folder. The second project of
  a folder named `app` on one Host is called `app (2)`, a name no folder
  has; a Cloud project's folder is `~/projects/<id>`, and its name lives
  only in Demi.

## Decision

A project is a folder on a Host. What it is called is the folder's name,
and what Demi keeps for it, its settings and now its saved messages, stays
with it until the folder's project comes back.

A project keeps its saved messages in one of two ways, which the user picks
on the project:

* **Per Conversation**, as today and the default: each conversation has its
  own list.
* **Per Project**: every conversation of the project shows one list. A
  message saved in one conversation is in the Saved list of all of them,
  and a message sent or deleted from one leaves all of them.

The project header's context menu holds the choice, as a submenu with a
check on the current way:

```text
┌──────────────────────────────┐
│ New Conversation Here        │
│ Fold                    ↩    │
│ Select Conversations         │
│ Saved Messages             › ├──┬─────────────────────┐
├──────────────────────────────┤  │ ✓ Per Conversation  │
│ Delete Project…              │  │   Per Project       │
└──────────────────────────────┘  └─────────────────────┘
```

### Switching moves the messages

* **Per Conversation to Per Project** merges at once, without a question:
  the messages of every conversation of the project, archived ones
  included, become the project's list, newest first by when each was
  saved. Nothing is lost and nothing is duplicated.
* **Per Project to Per Conversation** asks which conversation receives the
  project's messages, since a message can be in one list only:

  ```text
  ┌────────────────────────────────────────────────┐
  │ Keep Saved Messages per Conversation?          │
  │                                                │
  │ The project’s 5 saved messages move to one     │
  │ conversation.                                  │
  │                                                │
  │ Move to: [ Fix the cart total            ⌄ ]   │
  │                                                │
  │                         [ Cancel ]  [ Move ]   │
  └────────────────────────────────────────────────┘
  ```

  The pop-up lists the project's conversations that are not archived, the
  most recently changed first; it starts on the conversation shown when
  that one is in the project. Every other conversation starts with an
  empty list. When the project's list is empty, the switch asks nothing.
  When it has messages and no conversation that is not archived, Per
  Conversation is disabled, with the reason as its tooltip: there is no
  conversation to keep them.

### The other ways messages change hands

A message is always in exactly one list, and nothing ever deletes one but
the user:

* A conversation that moves into a Per Project project, by a change of
  where it runs, brings its own messages into the project's list.
* A conversation that leaves a Per Project project takes none of the
  project's messages, and starts with an empty list of its own.
* A deleted project keeps its list, as it keeps everything else; see
  [Deleting a project keeps it](#deleting-a-project-keeps-it).
* A project that goes into another, when its Host names their directories
  as one or its directory is a worktree of the other's repository, takes
  the receiving project's way. Its Per Project list joins the receiving
  project's list when that one is Per Project too. Otherwise it goes to
  its most recently changed conversation, else to the receiving project's,
  else into the receiving project's own list, which shows once that
  project is Per Project. A deleted project that receives one comes back.

### Deleting a project keeps it

Today Delete Project archives the project's conversations and deletes the
project itself. Restoring one of those conversations makes a new project
for its directory, which starts over: Alex's choice of model and workflow
for the project are gone, and so would be its saved messages.

Delete Project now marks the project deleted instead:

* It leaves the sidebar, the directory picker and `demi conversation
  projects`, and its conversations are archived, as today. Nothing else of
  it changes: its name, its settings, its Saved Messages choice and its
  saved messages stay.
* It comes back when the user adds its directory on its Host again, by
  starting or moving a conversation there, or restores one of its
  conversations. It comes back whole, with all it kept, at the top of the
  projects, where a new project goes.
* The delete dialog says nothing new: what it says, that the conversations
  are archived and the files stay, is still all that happens.
* A project that goes into another, when its Host names their directories
  as one or its directory is a worktree of the other's repository, is
  still removed for good, since its conversations all go into the other.

### A project is named by its folder

* A project's name is its folder's name, read from its path, never stored:
  `/Users/alex/work/app` is `app`. A folder that is a disk's root is named
  by its path. When the Host spells the folder otherwise, the name follows.
* Two projects of one name on one Host are told apart as two editor tabs
  of one file name are in VS Code: after the name, in faint text, the
  shortest end of their parent folders that differs, and the whole path in
  the tooltip. Projects of one name on two Hosts name their Host there, as
  they do today.

  ```text
  app   work                      /Users/alex/work/app
  app   old                       /Users/alex/old/app
  api                       ☁     (one of its name: nothing after it)
  ```

  The folder picker of a conversation lists each project with where it
  is already, so it needs nothing more.
* New Project on the Cloud makes the folder `~/projects/<name>`, so the
  name the user types is the folder's. A name a folder there has already
  is refused, as Finder refuses it: "The name “app” is already taken.
  Choose a different name."
* `workspaces.name`, the name of `PATCH /workspaces/:id` and the name of a
  device project's `POST /workspaces` go away. The web API's project keeps
  `name`, which the backend reads from the path.
* An existing Cloud project keeps its folder, `~/projects/<id>`, and is
  named by it from now on: no file on the Cloud is moved.

### Limits

The limit of 200 messages applies to each list, project or conversation,
because the composer receives the list whole with the conversation. A
merge never drops a message: a list a merge pushes past 200 keeps all of
them, and refuses a new save until the user sends or deletes enough.

### Design

* `workspaces` gains `saved_scope` (`'conversation'` or `'project'`, default
  `'conversation'`).
* `saved_messages` belongs to a conversation or to a project:
  `conversation_id` becomes nullable and `workspace_id` is added, with a
  check that exactly one is set. A migration rebuilds the table; every row
  keeps its conversation.
* Every route and socket frame keeps naming the conversation the page shows
  (`/conversations/:id/saved/...`, `send_saved`). The backend resolves the
  list from it: the project's when the conversation's project is Per
  Project, otherwise the conversation's own. An archived conversation
  still refuses changes, though the list it shows is shared.
* The list's revision stays per conversation, on the conversation's draft
  row, so the page keeps one record per conversation and its revisions
  only grow. A change of a project's list raises the revision of each of
  the project's conversations in the same transaction, and marks each
  one's metadata, so every open page of the project receives the new list.
  A switch and a move between projects raise them too.
* `PUT /workspaces/:id/saved-scope` with `{ scope: 'project' }`, or `{ scope:
  'conversation', to: <conversation id> }`, which is required when the
  project's list has messages. The switch and its move are one
  transaction; a `to` outside the project, or archived, is refused.
* The web API's project carries `savedScope`, and `savedCount` for the
  dialog, through the existing sync of the projects.
* `workspaces` loses `name` and gains `deleted_at`, null for a project in use. Every read
  of the projects a user sees skips deleted ones; finding the project of a
  directory finds a deleted one too, and clears its `deleted_at`. The
  unique index on the user, the device and the directory holds deleted
  projects too, so one directory is never two projects.
* Old data meets the default, Per Conversation, which is what it had. An
  older page reads the list as before: it names only the conversation.

## Prior art

ChatGPT's and Claude's projects share instructions and files among a
project's chats, which is the precedent for a project-wide list; neither
has saved messages. VS Code's settings choose a scope, user or workspace,
for each setting. None of them moves data when the scope changes, since
their scopes are layered; Demi's list is one place, so the switch moves the
messages instead of hiding them.

## Alternatives

* Both lists at once: the composer shows the project's messages and the
  conversation's own in two sections, and Save for Later asks where. It
  answers a question on every save that the user answers once per project
  here.
* A switch that hides the other list instead of moving it: messages that
  are no longer visible anywhere look lost, and come back unexpectedly
  when the user switches again.
* An account-wide list for every project: the user asked for a project's.
* Keeping the stored name and numbering namesakes, `app (2)`: the name is
  then neither the folder's nor one the user chose.
* Deleting a project's list with it, or moving it to one of its archived
  conversations: the first loses what the user wrote when the conversations
  stay, and the second puts it in a conversation the user cannot predict.

## Open questions

None.
