---
status: proposed
---

# Changes picks the repository

## Problem

Alex works on a checkout that holds several repositories, such as one
`gclient` makes: `~/Projects/atlas` is a plain folder, `src` inside it is
a repository, and `src/engine` inside that is another. Alex's
conversation runs in `~/Projects/atlas`, because its build scripts expect
that directory. The agent edits files in `src/engine`, but the Change
view's Commits mode shows only "Not a Git repository": Demi reads Git
only at the working directory, and that directory is no repository. Alex
cannot see the uncommitted changes or the history of either repository,
and moving the conversation into `src/engine` would break the agent's
commands.

## Decision

In Commits mode, a repository menu sits in the toolbar, left of the
branch menu. It names the repository shown and lists the repositories
found for the conversation's working directory; choosing one shows that
repository's uncommitted changes, its files and its commits.

```text
┌───────────────────────────────────────────────────────────────────┐
│ ‹ ›  [Commits] Conversation        ⛁ src/engine ⌄   ⑂ main ⌄   ⊟ │
├─────────────────────────────────────┬─────────────────────────────┤
│ src/render/frame.cc  +4 −1          │ src/engine                  │
│                                     │  ▾ src/render               │
│  12  void Frame::Draw() {           │      frame.cc       +4 −1 M │
│  13 -  Clear();                     ├─────────────────────────────┤
│  13 +  Clear(background_);          │ ● Uncommitted Changes     1 │
│                                     │   Draw the frame's backg…   │
└─────────────────────────────────────┴─────────────────────────────┘

The menu opened:

                          ┌──────────────────────────┐
                          │ ⌕ Search repositories    │
                          ├──────────────────────────┤
                          │ ⛁ src                    │
                          │ ⛁ src/engine           ✓ │
                          │ ⛁ tools/gn               │
                          └──────────────────────────┘
```

- **What the menu lists.**
  - The repository that holds the working directory, if any.
  - Every repository up to two levels below the working directory. The
    scan does not enter `.git` or `node_modules` folders, and does not
    follow symbolic links. It does enter a repository, so it finds
    `src/engine` inside `src`.
  - The repository of each file an agent of the conversation changed
    inside the working directory, at any depth, as the page's transcript
    holds them.
  - The directory a commit link or a subagent's branch opened the view
    on, when it is not one of the above.
- **Names and order.** A repository below the working directory is
  named by its path from it, `src/engine`. The working directory's own
  repository, and a directory a link opened, are named by their folder's
  name: `shop` for a conversation in `~/Projects/shop`. The working
  directory's own repository comes first, then the others by name.
- **What shows by default.** The repository of the file an agent of the
  conversation changed most recently. Without such a file, the working
  directory's repository. Without one, the first repository found. With
  none, Commits says "Not a Git repository", as today, with no menu.
- **A pick stays.** A repository the user picks stays shown when the
  agent later changes files elsewhere. The menu has no "Automatic" item:
  a new Change tab starts from the default again. Picking a repository
  shows its Uncommitted Changes and its checked-out branch, and is no
  step of Back and Forward, as choosing a branch is not. Picking a
  commit or a branch in the repository shown by default keeps that
  repository too, so the commit stays shown when the agent goes on in
  another one.
- **Layout.** The menu shows whenever Commits has a repository to show,
  one included, so the toolbar does not change as the list grows. While
  the list is on its way, the menu keeps its room and the view waits in
  its loading state, reading nothing, rather than showing the working
  directory first and then jumping to another repository. The Host is
  asked only while a Change view or its badge is on screen.
- **Scope.** Only Commits and its sidebar, the files tree and the commit
  list, change. Conversation mode and the File view stay as they are.

The design to build from:

- The Host lists the repositories in one request, `git_repositories`
  (`crates/runner-host/src/git.rs`), which finds each with the same
  `discover` as the working-tree requests:

  ```rust
  /// The repositories at and below `root`, and those holding `paths`.
  Inbound::GitRepositories {
      id: String,
      /// The conversation's working directory.
      root: String,
      /// Directories of the files the agents changed, each inside `root`.
      paths: Vec<String>,
  }

  pub struct GitRepositories {
      /// Whether `root` lies in a repository, at it or above it.
      pub in_repository: bool,
      /// The work trees below `root`, by absolute path, spelled from
      /// `root` as the request named it.
      pub repositories: Vec<String>,
  }
  ```

  The backend serves it to the page as
  `POST /conversations/{id}/changes/repositories`, with the paths in the
  body, beside `changes/branches`.
- The conversation's file service (`packages/web/src/conversation/files.ts`)
  owns the list. It collects the directories of the files the
  conversation's agents changed from the transcripts the page holds,
  asks the Host again whenever that set gains a directory, and gives the
  view the named list and the default:

  ```ts
  interface Repositories {
    /** The working directory's own first, then the others by name. */
    list: readonly { root: string; name: string }[]
    /** The repository of the most recently changed file, else the first. */
    preferred: string | null
  }
  ```

- What the view shows stays `ChangeData.root`. A pick writes the
  repository's path to it, or null for the working directory's own
  repository. When `root` is absent and the request names no directory,
  the view shows `preferred`, which is derived each time and never
  stored. The address carries the pick as it carries `root` today.
- The wire version rises to 68: an older runner is refused and updates
  itself. An address from before, which names no pick, shows the
  default.

## Spike

None: each piece reuses a working one. Commits already reads the
working tree and history of any directory `root` names, and `discover`
already finds the innermost repository holding a path.

## Prior art

- VS Code's Git extension
  ([`model.ts`](https://github.com/microsoft/vscode/blob/main/extensions/git/src/model.ts),
  `traverseWorkspaceFolder`) opens every repository in a workspace
  folder up to `git.repositoryScanMaxDepth` levels (1 by default),
  skipping `git.repositoryScanIgnoredFolders` (`node_modules` by
  default), and with `git.autoRepositoryDetection` also the repository
  of each open editor at any depth, and submodules up to
  `git.detectSubmodulesLimit` (10). Demi scans two levels, because a
  checkout such as `gclient`'s nests its repositories one level deeper
  than the folder the build runs in; the files the agent changed stand
  in for open editors.
- Zed's Git panel
  ([`git_store.rs`](https://github.com/zed-industries/zed/blob/main/crates/project/src/git_store.rs),
  [`workspace.rs`](https://github.com/zed-industries/zed/blob/main/crates/workspace/src/workspace.rs))
  keeps one active repository, which follows the focused editor's file,
  the innermost repository winning; its picker names a repository by its
  folder's name and sorts by it. Demi follows the agent's latest edit
  the same way, and departs from Zed in one point: a repository the user
  picks is not replaced by the next edit, since the agent's edits come
  without the user's focus moving.

## Alternatives

- **Move the conversation into the repository.** The agent's commands
  run from the working directory, and the build scripts of such a
  checkout expect the outer folder.
- **Show every repository at once, as VS Code's Source Control view
  does.** The Change view's sidebar holds one files tree and one commit
  list; stacking several would bury the one the agent works in, and the
  commit list would need a repository for every row.
- **Ask the backend for every file the conversation ever changed.** The
  page holds only the transcript windows it loaded, so a repository the
  agent changed long ago and deeper than two levels may be missing from
  the menu. Reading the whole stored transcript at each open costs more
  than that miss; the scan still finds every repository near the working
  directory.

## Open questions

None.
