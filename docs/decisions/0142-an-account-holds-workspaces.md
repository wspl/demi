---
status: proposed
---

# An account holds workspaces, each with its own conversations and projects

## Problem

Alex uses one Demi account for his own projects and for his job. The
sidebar lists both together: a conversation about his garden planner sits
between two about his employer's billing service, and Search, Archived and
the schedules mix them too. The only way to separate them today is a
second account, which also means a second copy of every Host, model,
credential and instruction.

## Decision

An account holds one or more workspaces. A workspace has its own
conversations and projects; everything in Settings is the account's and
the same in every workspace.

| Each workspace has its own | Every workspace shares |
| --- | --- |
| Conversations, pinned and archived | General, Account, Notifications, Keyboard |
| Projects | Hosts and devices, port forwards |
| Search results | Models and providers |
| Schedules (they belong to conversations) | Credentials |
| Saved messages, drafts, shares | Personal instructions, subagents |
| | Plugins and skills |

The separation is for order, not for security. An agent still sees every
workspace: `demi conversation list` and `demi conversation projects` list
all of them, with a column naming each one's workspace, and `send` reaches
any conversation.

### The switcher

The switcher is its own row under the app's name at the top of the
sidebar. It shows the workspace's tile, its name, and a dot when another
workspace has a conversation that waits for the user, the same states
that light a row's dot: a permission request, or a finished or failed
answer not yet read.

```text
┌──────────────────────────────┐
│ demi                     [▤] │
│ ┌──────────────────────────┐ │
│ │ [▣] Personal         •  ⇅│ │   the switcher
│ └──────────────────────────┘ │
│  ✎ New              ⇧⌘O      │
│  ⌕ Search            ⌘K      │
│  ...                         │
└──────────────────────────────┘

  ┌────────────────────────────┐
  │ [▣] Personal         ⌃1  ✓ │   its menu
  │ [▣] Work           •  ⌃2   │
  │ ────────────────────────── │
  │ +  New Workspace           │
  │ ⚙  Workspace Settings…     │
  └────────────────────────────┘
```

- A workspace's tile is its icon, white on its color: one of 24 icons
  (a person, a briefcase, a house, a flask, …) and one of eight colors.
- **New Workspace** creates one at once, named *Workspace 2* (then
  *Workspace 3*), with the next icon and color, switches the tab to it,
  and opens it in Settings › Workspaces with its name selected, ready to
  type over.
- **Workspace Settings…** opens Settings › Workspaces, in the first group
  after Account. It reads as Providers does: the list of workspaces on
  the left, in the menu's order, which dragging changes, with New above
  it; the selected one on the right, its name over Delete as an icon,
  which asks first and deletes its conversations and projects with it,
  and is disabled for the last workspace. Under them a card: Name, a
  field; Icon, the tile, which opens the eight colors over the 24 icons;
  and Contents, how many conversations and projects it holds.
- ⌃1 to ⌃9 on a Mac, and Ctrl+Shift+1 to Ctrl+Shift+9 elsewhere, switch
  to the first nine workspaces in the menu's order. ⌘1 to ⌘9 and Ctrl+1
  to Ctrl+9 are the browser's own tab switching, which a page must not
  take; Arc switches its spaces with ⌃1 to ⌃9. Alt+1 to Alt+9 would be
  Chrome's and Firefox's tab switching on Linux, and ⇧⌘3 to ⇧⌘5 are the
  Mac's screenshots. Keyboard settings list them as nine shortcuts, Switch
  to Workspace 1 to 9, and change them as any shortcut.

### Each tab has its own workspace

A tab's workspace is the workspace of the conversation it shows.
Conversation IDs are unique across workspaces, so `/chat/<id>` names its
workspace, and no address changes. Alex can show a Personal conversation
in one tab and a Work conversation in another; each sidebar lists its own
workspace, and switching in one tab leaves the other as it is.

- **Switching** shows the conversation this browser last showed in that
  workspace, or else its first in the sidebar, or a new conversation when
  it has none, as Slack returns to a workspace's last channel.
- **A new conversation** has its ID before its first send, and its
  history entry already keeps the project it was started in; it keeps the
  workspace beside it, so a reload stays there.
- **An address that shows no conversation,** such as `/`, `/chat` or
  `/settings/models` opened in a new tab, opens the workspace the user
  last switched to in this browser.
- **A link, a notification or a search result** that opens a conversation
  of another workspace switches only the tab it opens in.
- **A conversation moved** to another workspace while a tab shows it takes
  the tab with it, as a Notion page moved to another workspace does.

The address does not say the workspace; the switcher does.

### Moving between workspaces

A conversation's menu and a project's menu gain **Move To ›**, whose
submenu lists the workspaces, the current one checked. It acts on every
selected conversation.

One folder may be a project in several workspaces: a project is a
workspace's folder on a device.

- **A conversation in a project** moves into the target workspace's
  project of the same folder, which is created when there is none. Alex
  moves one of five conversations of `~/code/acme-app` to Work: Work gains
  an `acme-app` project with that conversation, and Personal's keeps the
  other four.
- **A project** moves with all its conversations, archived ones included.
  When the target workspace already has a project of the folder, the two
  merge, as two projects that turn out to be one folder merge today: the
  conversations join it, the moved project's shared saved messages join
  its list, and the target's own choices (its saved messages' scope, its
  last model and workflow) stay.
- A project left without a conversation leaves the sidebar, as any such
  project does.

### Design

**The rename.** `workspace` names a project throughout the code and its
data today (the `workspaces` table, `WorkspaceId`, `/workspaces/:id`,
`target_kind = 'workspace'`, the context block's `workspace "acme-app"`),
and Settings has a group called Workspace. A first work package renames
each of them to `project` and changes nothing else; this record's work
package then gives the name to the new concept. Settings' Workspace group
becomes **Devices & Data** (Devices, Port Forwards, Archived, Data &
Privacy), and Keyboard joins the first group.

**Storage.**

```sql
CREATE TABLE workspaces (
  id         TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users (id),
  name       TEXT CHECK (name <> ''),  -- none: the first one's name
  icon       TEXT NOT NULL,
  color      TEXT NOT NULL,
  sort_order INTEGER NOT NULL,
  created_at INTEGER NOT NULL
) STRICT;

-- projects
workspace_id TEXT NOT NULL REFERENCES workspaces (id),
UNIQUE (workspace_id, device_id, path),  -- was (user_id, device_id, path)
UNIQUE (id, workspace_id),

-- conversations
workspace_id TEXT NOT NULL REFERENCES workspaces (id),
FOREIGN KEY (target_project_id, workspace_id)
  REFERENCES projects (id, workspace_id),
```

The composite key keeps a conversation in its project's workspace; SQLite
does not check it while `target_project_id` is null, a conversation
outside projects. The migration gives every user one workspace, holding all their
conversations and projects, and a new account starts with one. Its name is
null until the user names it, and each page says *Personal* in its own
language, as *个人* in Chinese; Demi's commands say *Personal*.

A git repository's linked worktrees are a fact of its folder, which may
now be a project in two workspaces. The runner's report is kept by device
and folder, in `directory_worktrees (device_id, path, name,
worktree_path)`, in place of `project_worktrees` by project, and every
project of the folder reads it.

Finding the project of a folder takes the workspace: each caller starts
from a conversation, which has one.

**Web API.**

```ts
interface Workspace {
  id: string
  name?: string          // absent: the name an account starts with
  icon: WorkspaceIcon    // one of the 24 icons' names
  color: WorkspaceColor  // one of the eight colors' names
}
// Conversation and Project gain `workspaceId: string`.

POST   /workspaces                  // { name }, the page's default → Workspace, next icon and color
PATCH  /workspaces/:id              // { name?, icon?, color? }
PUT    /workspaces/order            // { ids }
DELETE /workspaces/:id              // refused for the last one
POST   /conversations/move          // { ids, workspaceId }
POST   /projects/:id/move           // { workspaceId }
GET    /search?...&workspace=<id>
```

The page's state gains the account's workspaces, as one more part of the
one state. The page still receives every conversation and project of the
account through one connection, and each tab filters its sidebar,
Archived and schedules by its workspace; the switcher's dot reads the
other workspaces from the same state. The last workspace switched to is
kept in the browser with the account's other local data, which a
sign-out clears.

**What older pieces meet.** An older page stops connecting when the
backend names another build, and reloads into this one. Runners never
named projects, so none changes.

## Spike

None.

## Prior art

- **Linear** keeps several workspaces under one account, switched from the
  menu at the top left; each has its own teams and issues
  ([docs](https://linear.app/docs/workspaces)).
- **Slack** lists a user's workspaces in its switcher and marks one with
  unread activity; ⌘1 to ⌘9 switch in its desktop app only
  ([shortcuts](https://slack.com/help/articles/201374536-Slack-keyboard-shortcuts)).
- **Notion** creates workspaces under one account and moves a page
  between them.
- **Arc** separates work from personal browsing with Spaces, which share
  everything but their tabs, and switches them with ⌃1 to ⌃9
  ([help](https://resources.arc.net/hc/en-us/articles/19228064149143-Spaces-Distinct-Browsing-Areas)).
- **ChatGPT** and **Claude** switch workspaces from the account's menu, and
  the switch holds for every tab.
- **ChatGPT**'s projects take an icon and a color from a picker of each,
  as a workspace's tile does here.
- **VS Code** lets one folder belong to several multi-root workspaces.

Demi departs from Linear, Slack and Notion in two ways: a workspace is the
user's own, with no members or billing, and Settings are the account's
rather than each workspace's.

## Alternatives

- **The workspace in the address,** `/work/chat/<id>`, as Linear and
  Vercel do. A conversation's ID already names its workspace, while a
  readable name in the address needs a field of its own for names such as
  *个人*, changes with a rename, and must avoid the app's own paths.
- **Its ID in the address,** as Slack does. It says nothing the
  conversation's ID does not.
- **A second account,** as ChatGPT's account switching does. Every Host,
  model and credential would be set up twice.
- **One folder in one workspace only.** A conversation of a project could
  not move alone, and adding the folder in another workspace would be
  refused; the uniqueness key changes either way.
- **Settings for each workspace.** The user asked for one set.

## Open questions

- Settings' Workspace group renamed to Devices & Data, with Keyboard moved
  to the first group?
- The shortcuts ⌃1 to ⌃9 on a Mac and Ctrl+Shift+1 to Ctrl+Shift+9
  elsewhere?
- Does a moved conversation take the tab that shows it to its new
  workspace?
