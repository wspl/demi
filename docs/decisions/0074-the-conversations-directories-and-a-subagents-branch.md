---
status: proposed
---

# The Files tab switches between the conversation's directories, and a subagent's bar shows its branch

## Problem

A conversation's main agent works in `~/Projects/app`, and two subagents
each work in a checkout of their own, `~/Projects/app-1` and
`~/Projects/app-2`, on a branch of their own. The Files tab shows one
directory, and reaching another takes a subagent's window and its folder.
Which branch a subagent works on, and what it changed there, shows
nowhere until the user opens Changes and picks the folder and the branch.

## Decision

**The Files tab's directory is a menu.** The directory's name at the top of
the tab gets a chevron at its right, and the two are one button. Its menu
lists the directories the conversation's agents work in: the main agent's
first, then each running subagent's on the conversation's path, in the
order they were spawned, then the directory the tab shows when it is none
of these, each directory once. Choosing one shows it in the tab; the one
shown is checked. With one directory in all, the name stays plain text.
The directory shown stays in the menu after its subagent stops, so the
user can always go back to another.

```text
  app-1  ⌄                              ⟳
  ┌──────────────────────────────────┐
  │   app            ~/Projects       │
  │ ✓ app-1          ~/Projects       │
  │   app-2          ~/Projects       │
  └──────────────────────────────────┘
```

**A subagent's bar shows its branch.** After the folder, the bar of a
subagent's window shows the branch checked out in its folder, with the
branch icon, or `HEAD (3f2a9c1)` while HEAD is detached; nothing when the
folder is in no repository or its Host is away. Pressing it opens Changes
in Commits mode on that folder and that branch: its commits, and its
uncommitted changes. Where the room runs out, it gives way before the
folder's name.

```text
  Running · 2m  📁 app-1  ⑂ feat/login  Implement · GPT-6.1-Sol · High
```

The design to build from:

* The menu's directories come from the agents the page already holds: the
  main agent's directory and each running subagent's, the same that the
  bar's folder shows. Nothing new is read for it.
* The branch is the answer of `GET /conversations/:id/changes/branches`
  for the folder, its checked-out branch or detached HEAD, kept current as
  the Change view's history is: it follows the repository's HEAD, so a
  checkout from any program shows. A failed read shows no branch.
* Pressing the branch opens Changes with `mode: 'commits'`, `root` the
  folder and `branch` the one shown, as a commit link opens it today.
  It is a `branch` intent of its own, which the Changes plugin handles, as
  the `commit` intent is.
* The gallery's Files and Agents window specimens show the menu and the
  branch, and each responds when used.

The interfaces, in `web-ui`:

```ts
// files/types.ts: a directory a view can show among several
export interface FileDirectory {
  path: string
  /** What stands for its own name, such as the Cloud's Workspace. */
  name?: string
}

// FileTree's and the File view's props: the menu's directories
directories?: readonly FileDirectory[]

// markdown/types.ts, ConversationFiles: what a subagent's bar reads
history?(root: string): CommitHistorySource | null
openBranch?(target: MessageBranchTarget): void

// plugins: the intent the branch opens the Change view by
interface IntentPayloads {
  /** Null is a detached HEAD. */
  branch: { root: string; branch: string | null }
}
```

The page's `ConversationFileService` gives `directories`, the main agent's
and its running subagents' on the path, ordered by their number. A popover
takes `align`, the trigger's edge it keeps to, held inside its pane, which
the menu uses so that it opens toward the panel and stops at its edge.

## Spike

None.

## Prior art

* VS Code shows the branch in its status bar, and Zed and JetBrains IDEs
  in their title bars; pressing it opens a picker that checks out another
  branch. Demi's opens the branch's history in the Change view instead: a
  subagent's branch is the subagent's to change, and what the user wants
  of it is what the subagent did there.
* The Files menu is the composer's directory menu, row for row, without
  its Choose Another Directory item.

## Alternatives

* **The menu in Changes too.** The branch opens Changes on a subagent's
  folder already, and a commit link on its writer's.
* **One tree with a root for each directory,** as VS Code's multi-root
  workspace shows them. Checkouts of one repository hold the same files,
  so the tree would repeat every folder once per agent.

## Open questions

None.
