# File browser

`web-ui/files` is the one chooser for a folder or a file on a host: the workspace
picker (create or move a workspace), the new-project form's Browse, and the
composer's remote attachment all open it. It is laid out like the Windows open
dialog because that is the shape people already know for the job.

```text
┌ Select folder ─────────────────────────────────────────────── × ┐
│ [🖥 zan-mbp      ˅]  ‹ › ↑ [ 💽 › Users › zan › Projects ] [＋] [👁] │
├───────────────┬─────────────────────────────────────────────────┤
│ QUICK ACCESS  │ Name ˄              Date modified          Size │
│  ⌂ Home       │ 📁 assetsfactory    Sep 2, 18:12               │
│  📁 Desktop   │ 📁 demi             17:12                      │
│  📁 Projects  │ 📁 wynk             2025-12-30                 │
│ RECENT        │ 📄 notes.txt        Sep 6, 10:12         512 B │
│  ◷ demi       │                                                │
├───────────────┴─────────────────────────────────────────────────┤
│ Selected folder: 📁 demi               [Select Folder] [Cancel] │
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
sidebar's `places` (groups of paths with a label and icon) and `hosts` (the devices
the address bar's picker offers, each with an online dot). The browser owns
navigation, history, selection, sorting and the hidden-files switch; it emits
`select` with one absolute path and `cancel`. Choosing another device emits
`update:hostId`; the caller answers with that device's source, and the browser
starts over at its home (or the caller's new `initialPath`). `FileBrowserDialog`
wraps the browser in a `lg` dialog under a plain title bar, at a fixed height so the
list has room whatever the folder holds.

## Behavior

- **Device and address**: two controls. The device sits over the rail, as wide as it,
  a menu of the hosts the caller offers, dotted green or gray; then Back, Forward and
  Up, and the path from that device's root, as crumbs, each a jump; the root crumb is
  a drive glyph, since a lone slash is too narrow to hit. Deep paths keep the root and the last three crumbs (two in a
  narrow bar) and fold the middle into an ellipsis. A click on the free space turns
  the bar into a text field with the full path; Enter goes there, Escape or blur
  leaves it.
- **List**: Name, Date modified and Size; folders first, names in natural order
  (`file2` before `file10`). A header click sorts ascending, a second descending, a
  third returns to the default order; every column shows a faint pair of chevrons
  until it is the sorted one. A click selects; a double click or Enter opens a folder
  or, in file mode, confirms a file. In folder mode files are shown dimmed and cannot
  be picked. Hidden entries (dot names) show only with the switch on.
- **Keys** in the list: arrows move the selection, Home and End jump, Backspace goes
  up, ⌥← and ⌥→ walk the history.
- **Toolbar icons**: after the address bar, New folder and the hidden-files switch
  as icon buttons with tooltips; the switch shows pressed while hidden files show.
- **Status row**: gray words, then the entry in white with its icon: "Selected
  folder: 📁 demi", "Selected file: 📄 README.md", or with nothing selected "Select
  a folder, or use 📁 Desktop" since in folder mode the confirm button takes the
  current folder, and "Select a file" in file mode. A failure to create a folder
  reads here in red. On the right the confirm button and Cancel.
- **New folder**: an editable row at the top of the list with the name ready to type
  over; Enter creates it and selects it, Escape or blur drops it.
- **States**: a spinner while a folder loads, one line for an empty folder, and for
  a failure an icon with the reason.
- **Narrow**: below a phone width the path takes a row of its own under the toolbar,
  whose left keeps the device and nav and whose right holds a Places menu with the
  rail's groups beside the icons; Forward goes, the date column hides and the
  buttons drop under the status.

Nothing in the browser is document text: the whole surface is `select-none` and the
only text cursor is inside the address field and the new-folder name.

## Where it opens

| Surface | Mode | Source | Places | Hosts |
|---|---|---|---|---|
| Workspace control, "Choose another directory…" | directory | the conversation's main device | Home, the device's workspaces | every device and Cloud |
| Main host switch (host menu) | directory | the chosen device | same | same |
| New project, Browse… (a page of the same dialog) | directory | the form's device | same | the user's devices |
| Composer Add, "Attach remote file…" | file | the conversation's main host | Home, the device's workspaces | main and attached hosts |

The composer's Add menu shows "Attach files" alone for a conversation without a host,
and "Attach local files" beside "Attach remote file…" when it has one. A chosen remote
file's path goes into the draft as a text reference; the agent reads it on the host.
