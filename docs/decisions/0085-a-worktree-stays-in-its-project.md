---
status: proposed
---

# A conversation in a worktree stays in its repository's project

## Problem

The user's project `app` is `~/Projects/app` on their Mac. An agent of a
conversation in it moves itself to `~/Projects/app-worktrees/1`, a linked
worktree of the same repository that `git worktree add` made, with
`demi host move`. A directory that is no project and not the Host's home
becomes a project of its own, so the sidebar gains a project named `1`, the
conversation leaves `app` for it, and the header shows `1`. The user still
works on `app`, and finds the conversation under a name that says nothing.

## Decision

**A linked worktree's conversation stays in the project of its main
worktree.** When a conversation's directory is the root of a linked
worktree, and the main worktree of its repository is a project on the same
Host, the conversation is in that project and works in the worktree. A
worktree's directory that was a project of its own goes into the
repository's project. A worktree whose main worktree is no project becomes
a project as any directory does.

**The worktree's name is git's.** It is the name git gives the worktree
when it adds it, the name of its directory under `.git/worktrees/`: the
last part of the worktree's path, with a number after it when another
worktree of the repository has that name (`feature`, `feature1`).

**The sidebar marks the row.** The conversation's row stays in its
project, and its status dot stays where it is. A worktree mark joins the
row's trailing marks, after the schedules' alarm clock; its tip and its
accessible name are *Worktree 1*. A row in a project shows no device's
mark, so the two never meet. The mark is VS Code's worktree codicon, read
from `@vscode/codicons` under CC BY 4.0, whose notice the release carries:
Lucide, the other glyphs' set, has none, and its branch, fork and folder
glyphs already mean other things here.

```text
  app
    ●  Fix the sign-in redirect
    ●  Design the empty state              ⑂
    ●  Tidy the settings page          ⚡  ⑂
```

**The header shows it.** After the project's directory button, the header
shows the worktree's mark and name, as a label: it opens nothing, and its
tip is the worktree's path. The directory button shows the project, and
its menu lists the projects as it does today, worktrees none: a
conversation enters a worktree only by a move to its directory, by an
agent or Choose Another Directory, and choosing its project in the menu
moves it back to the project's directory.

```text
  Design the empty state  ✎           MacBook Pro ⌄   app ⌄  ⑂ 1
```

**The Workflow menu offers the project's workflows, and the worktree's
when they differ.** A conversation in a worktree uses the workflows of its
project's directory, the nearest `.demi/workflows/` from there up to the
repository's root, as a conversation in the project does. A worktree checked
out at an older commit, or on a branch without a workflow file, still offers
the project's. When the worktree's own workflows differ from the project's,
the menu lists both, each under its directory's name, and the choice records
which one it is. The agent's commands see and pick the same:
`demi agent workflows list` marks the worktree's in its `worktree` column,
and `demi agent workflows use <id>` picks the project's, the worktree's with
`--worktree`, or the worktree's when only the worktree has that id.

```text
  Workflow ⌄
  ┌──────────────────────────────┐
  │   Global Workflow            │
  │   No Subagents               │
  │ ─────────────────────────────│
  │ app                          │
  │ ✓ Parallel Development       │
  │   Review Team                │
  │ Worktree 2                   │
  │   Parallel Development       │
  └──────────────────────────────┘
```

**Instruction files stay the worktree's.** `AGENTS.md` and `CLAUDE.md`
describe the code checked out where the agent works, as every agent tool
reads them. The context card already names each one's path in its tip.

The design to build from:

* **The backend knows every project's worktrees before a move.** Each
  runner reports the linked worktrees of the repositories of its Host's
  projects, and the backend stores them with each project, so a move reads
  them in its own transaction, with the Host asleep or away, and never asks
  the Host (0069). The backend gives the runner the directories of the
  Host's projects at its hello and again when they change.
* **A report comes before what follows the change.** The runner checks the
  repositories before it sends anything through which a change it made
  could lead to a move: its hello, a `demi` command's request, a job's end,
  and its answer to a folder's listing. A repository whose `.git`, its
  `worktrees/` folder or a worktree's `gitdir` file changed since its last
  report is read again with gix, which reads git's own files, relative
  worktree paths included, and its list is sent whole, on the same
  connection, before that message. So `git worktree add ../w && demi host
  move ../w` finds the worktree, and so does a move to `../w` after the
  job that added it ended, from the same Host or another.

  ```rust
  // runner-protocol, backend to runner
  Projects { directories: Vec<String> }
  // runner to backend
  Worktrees { directory: String, worktrees: Vec<LinkedWorktree> }
  pub struct LinkedWorktree {
      /// Its name under the repository's `.git/worktrees/`.
      pub name: String,
      /// Its canonical path.
      pub path: String,
  }
  ```

  A project's directory that is no repository's main worktree reports
  none.
* **One rule: a worktree of a project is never a project of its own.**
  - A move or a creation in a directory selects, in order: the project of
    that directory; else the project that lists it as a worktree, with the
    worktree; else a project made for it, as today.
  - When a project's worktrees are stored and one of them is a project of
    its own, that project goes into the repository's project, its
    conversations with it, as a second spelling of one directory does
    today. This covers a worktree a move reached before its report, and a
    project made for a worktree before this change.
  - A directory the Host names otherwise at its first use (0069) is
    selected again by the same rule under its canonical name.
* The selection keeps the worktree:

  ```rust
  Selection::Workspace {
      workspace_id: WorkspaceId,
      device_id: DeviceId,
      /// The project's directory, kept for a restore.
      path: String,
      /// The worktree it works in, in place of the project's directory.
      worktree: Option<Worktree>,
  }
  pub struct Worktree { pub name: String, pub path: String }
  ```

  The conversations table gains `target_worktree_path` and
  `target_worktree_name`, null by migration, and a `workspace_worktrees`
  table holds each project's list as last found. A conversation in a
  worktree resolves to the worktree's path, which its agents, shells and
  Files tab start in. One that git removes stays in its worktree, as a
  conversation stays in a directory that was deleted.
* The conversation the page reads gains `worktree: string | null`, the
  name; the sidebar's row model and the header take it from there.
* A workflow choice says which directory's file it is, and the
  conversation's metadata carries the worktree's list beside the project's
  while the two differ:

  ```rust
  // shared-types: the worktree's file rather than the project's; written
  // only when true, and a conversation in no worktree reads its own
  // directory's either way
  WorkflowChoice::Workflow { id: String, worktree: bool }
  // web-api-protocol
  pub struct ConversationWorkflowList {
      /// The sum of the two directories' revisions.
      pub revision: u64,
      pub list: KeptWorkflowList,
      /// The worktree's, while it differs from the project's.
      pub worktree: Option<KeptWorkflowList>,
  }
  ```

  The conversations table gains `workflow_worktree`, false by migration. A
  project's last choice, which its next conversation starts with, keeps no
  worktree, since a new conversation is in none.
* Restoring a conversation whose project was deleted selects its
  worktree's directory, which becomes a project as any directory does.
* An older runner is refused and updates itself; the wire version rises.

**What the order does not cover.**

* A folder the page lists over its direct link to the Host, as the work
  panel does: no report goes first. No move follows such a listing; Choose
  Another Directory lists through the backend, whose answer the report
  goes before.
* A worktree added while the Host's runner was away, and moved to before
  it is back: the move makes a project, which the hello's report folds in.

## Spike

Git's naming and what each of its worktree commands changes, run with git
2.54 on macOS (APFS), and what a check costs:

| Command | What changes |
|---|---|
| `worktree add ../a/feature`, then `add -b y ../b/feature` | `.git/worktrees/feature` and `.git/worktrees/feature1`; each one's `.git` file names its directory there |
| `add`, `remove`, `prune` | the `worktrees/` folder's modification time |
| `move`, `repair` | the worktree's `gitdir` file's time and text; its folder's time does not change |
| A check of 50 repositories, about 350 `stat` calls | 0.4 ms |

So the check stats `.git`, `worktrees/` and each `gitdir`, never the
worktree folders alone.

## Prior art

* The Codex app keeps a worktree's thread under its project in the
  sidebar, and its absence there was fixed as a bug
  ([openai/codex#10522](https://github.com/openai/codex/issues/10522)); it
  shows a worktree mark by the branch
  ([#13387](https://github.com/openai/codex/issues/13387)).
* VS Code lists a repository's worktrees under it in the Repositories view,
  each with a worktree icon
  ([microsoft/vscode#284224](https://github.com/microsoft/vscode/pull/284224)).
* Conductor nests each workspace, a worktree, under its repository.

Demi departs from all three in that a worktree is no place the user picks:
it is where an agent works, which the row and the header show.

## Alternatives

* **A project of its own, nested under the repository's.** Two levels in
  the sidebar for what is one project to the user, and the worktree's
  directory name stays its name.
* **The page tells the backend which worktree it chose.** The page picks
  no worktree, and an agent's move has no page.
* **The Host tells at the directory's first use,** in its answer to
  `FsDirectory`. A move stores the directory without asking the Host
  (0069), so the sidebar would show a project made for the worktree until
  the Host answered.
* **A file watch of each project's `.git/worktrees/`.** A move in the
  100 ms the watch gathers changes for (`WATCH_GATHER`) would come before
  the report and show a project for that time. On macOS each watch outside
  a working tree is a kqueue of its own, of which a connection keeps 64.
* **The move asks the Host when it is online.** A round trip for every move,
  and a Host that answers late holds the move, which 0069 removed.
* **The branch as the name.** A worktree with HEAD detached has none, and
  the branch is shown by the Changes view already.

## Open questions

None.
