# File browser

`web-ui/files` is the one chooser for a folder or a file on a host: the workspace
picker (create or move a workspace), the new-project form's Browse, and the
composer's remote attachment all open it. It is laid out like the Windows open
dialog because that is the shape people already know for the job.

```text
┌ Select folder · zan-mbp ───────────────────────────────────── × ┐
│ Conversations in this workspace run in the folder you choose.   │
│ ‹ › ↑  [ 🖥 zan-mbp › Users › zan › Projects        ]  [🔍 Filter] │
│ [＋ New folder]                              [ ] Hidden files    │
├───────────────┬─────────────────────────────────────────────────┤
│ QUICK ACCESS  │ Name ˄              Date modified          Size │
│  ⌂ Home       │ 📁 assetsfactory    Sep 2, 18:12               │
│  📁 Desktop   │ 📁 demi             17:12                      │
│  📁 Projects  │ 📁 wynk             2025-12-30                 │
│ RECENT        │ 📄 notes.txt        Sep 6, 10:12         512 B │
│  ◷ demi       │                                                │
│ DEVICES       │                                                │
│  ● zan-mbp    │                                                │
│  ● build-01   │                                                │
│  ○ studio     │                                                │
├───────────────┴─────────────────────────────────────────────────┤
│ Folder: [ demi                     ]   [Select Folder] [Cancel] │
└─────────────────────────────────────────────────────────────────┘
```

## Contract

The browser reads through a `FileBrowserSource` and never knows where entries come
from (`files/types.ts`):

- `home`: where it opens when the caller names no path, and where `~` and Home go.
- `list(path, signal)`: the entries of one directory — name, `isDirectory`, and size
  and modification time when the source knows them. A known failure rejects with a
  `FileBrowserError` of kind `not-found`, `permission`, `offline` or `other`; the
  kind picks the empty-state copy, the message is shown under it.
- `createDirectory(path)`, optional: without it there is no New folder button.

Paths are POSIX. The backend's device fs (`GET/POST /api/devices/:id/fs`) maps onto
this directly; prototypes and the gallery use `createMemoryFileSource` over a fixture
tree (`files/memory-source.ts`).

The caller also supplies `mode` (`file` or `directory`), an `initialPath`, the
sidebar's `places` (groups of paths with a label and icon) and `hosts` (devices with
an online dot). The browser owns navigation, history, selection, sorting, the filter
and the hidden-files switch; it emits `select` with one absolute path and `cancel`.
Choosing another host emits `update:hostId`; the caller answers with that host's
source, and the browser starts over at its home (or the caller's new `initialPath`).
`FileBrowserDialog` wraps the browser in a `lg` dialog with a title and one line of
description, at a fixed height so the list has room whatever the folder holds.

## Behavior

- **Address bar**: crumbs, each a jump; the root crumb is the host. Deep paths keep
  the root and the last three crumbs (two in a narrow bar) and fold the middle into
  an ellipsis. A click on the free space turns the bar into a text field with the
  full path; Enter goes there, Escape or blur leaves it.
- **List**: Name, Date modified and Size; folders first, names in natural order
  (`file2` before `file10`). A header click sorts, a second click flips; the time
  column starts newest first. A click selects; a double click or Enter opens a folder
  or, in file mode, confirms a file. In folder mode files are shown dimmed and cannot
  be picked. Hidden entries (dot names) show only with the switch on. The filter
  narrows the folder and highlights the match.
- **Keys** in the list: arrows move the selection, Home and End jump, Backspace goes
  up, ⌥← and ⌥→ walk the history.
- **Name row**: the selected row fills it; typing something else drops the selection.
  Enter resolves the text against the folder: a folder is entered, a file is opened
  in file mode, `~` is the home, a leading slash makes it a path. The confirm button
  does the same, except that in folder mode a typed folder is chosen rather than
  entered, and with nothing typed the current folder is the answer. What is not there
  says so under the row, as does a file typed where a folder was asked for.
- **New folder**: an editable row at the top of the list with the name ready to type
  over; Enter creates it and selects it, Escape or blur drops it.
- **States**: a spinner while a folder loads, one line for an empty folder or an
  empty filter, and for a failure an icon with the reason.
- **Narrow**: below a phone width the sidebar becomes a Places menu with the same
  groups, Forward goes, New folder is an icon, the filter moves to the second row,
  the date column hides and the buttons drop under the name.

## Where it opens

| Surface | Mode | Source | Places | Hosts |
|---|---|---|---|---|
| Workspace control, "Choose another directory…" | directory | the conversation's main device | Home, the device's workspaces | every device and Cloud |
| Main host switch (host menu) | directory | the chosen device | same | same |
| New project, Browse… | directory | the form's device | same | the user's devices |
| Composer Add, "Attach remote file…" | file | the conversation's main host | Home, the device's workspaces | main and attached hosts |

The composer's Add menu shows "Attach files" alone for a conversation without a host,
and "Attach local files" beside "Attach remote file…" when it has one. A chosen remote
file's path goes into the draft as a text reference; the agent reads it on the host.
