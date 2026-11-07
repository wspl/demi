---
status: proposed
---

# A conversation's work opens in windows on a taskbar

Supersedes:

- 0004 (a conversation's pictures and videos open in the work panel), its
  place: the Media tab opens in the Attachments window. The series, its
  controls and its keys still hold.
- 0014 (the work panel has addresses), the open and closed panel: `panel`
  now names the tab of the window in front. The places, the history steps and
  Back still hold.
- 0179 (a phone shows a panel view over the conversation), the overlay and
  the full panel: a phone shows one window at a time over the conversation.
- 0180 (a tab's menu closes every tab), Close All's pinned tabs: a window
  with one tab, such as Changes, has no Close commands for it.
- The dock's windows: Agents and Running over the composer, their chips, the
  height share they keep and their closing on a press outside them.

## Problem

Alex asks the agent to fix a failing test. The reply names `cookie.ts`, opens
the test page in a browser tab and starts a build. To look at the file, Alex
opens the work panel at the right; it takes 42% of the screen and stays,
narrowing the conversation, until Alex closes it again. The browser tab is
the panel's second tab, so seeing the file and the page together is not
possible. The build's output opens in another kind of window, Running, which
rises over the composer and closes at the next press outside it, and the
agent's subagent opens in a third, Agents, in the same place. Three ways to
show work, each with its own rules for where it stands, what closes it and
how big it is, and none of them shows two things side by side.

## Decision

**What the conversation's work shows opens in windows over and beside the
conversation, and a taskbar under them lists them.** Every window works the
same: Files, Changes, the browser, a device, Attachments, and the system's
Agents, Running and Services.

```text
 +---------------------------------------------------------------+
 | Fix the failing login test        device  project  worktree ... |  top bar
 +------------------------------------------+--------------------+
 |  conversation                            | [F] cookie.ts  x   |  docked
 |                +----------------------+  | ...                |  column
 |                | (G) Orders | Docs  +  |  +--------------------+
 |                | floating window      |  | [C] Changes        |
 |                +----------------------+  | ...                |
 |  +------------------------------------+  |                    |
 |  | message...                         |  |                    |
 |  | (+)  workflow  saved   model  (^)  |  |                    |
 |  +------------------------------------+  |                    |
 +------------------------------------------+--------------------+
 | +  [F] cookie.ts | (G) Docs 2  [A] chart.png  | Agents 3  Running 5 |
 +---------------------------------------------------------------+
   taskbar: +, pins | open windows              | system area
```

### Windows

- **One window per kind of work, tabs inside it.** The window's title bar is
  its tab strip, as in Chrome: the tabs, each with its page's icon or its
  kind's mark, a + after the last where the kind opens new tabs (the
  browser), room to grab, a divider, then Keep in Corner where the window
  may stand there, Minimize, Maximize and Close.
  - A tab grows in and shrinks out as Chrome's do. Its menu offers Close,
    Close to the Right, Close to the Left, Close Others and Close All; tabs
    that no longer fit are listed under Show All Tabs.
  - A window that shows one view, such as Files or Changes, has no tab: its
    title bar shows the view's icon and what it shows, such as
    `package.json`.
  - A window of tabs always has one selected; one whose last tab closes
    closes.
  - Files: the file viewer and its tree together, its Back and Forward
    walking the files it showed (0014).
  - Changes: the change with its commits.
  - Browser and Simulators: a tab per page or device.
  - Attachments: the Media series (0004) and each attachment opened.
  - Agents, Running and Services: a tab per agent or command, as their dock
    windows had.
- **A window stands in one of five places.**
  - Floating: where it was put, over the conversation. A window opens
    floating in the middle of the area above the composer, 60% of its width
    and 72% of its height, at most 1100 x 820 px and at least 480 x 360 px,
    so a look at a file or a picture leaves the conversation in sight. One
    that would open where another floats steps 24 px down and right, as
    Windows cascades them.
  - Anchored: the area's bottom right corner, above the system buttons,
    over the conversation and the composer's end. Agents, Running and
    Services open there. It does not move; its top and left edges size it.
    One window stands there at a time: one that comes minimizes the one
    there. Keep in Corner puts a floating window there and takes it out
    again.
  - Docked: a row of the column at the area's right, at most 3 rows, never
    columns; the conversation narrows beside it.
  - Maximized: the conversation's side of the area under the top bar, the
    composer included, 8 px inside its edges; a docked column stays beside
    it.
  - Minimized: only its taskbar button shows.
- **Moving.** Dragging the title bar's room moves a floating window, the
  pointer an arrow, never a hand. A window stays 8 px inside the area: an
  edge that reaches it stops, and the window narrows behind the pointer for
  as long as the drag lasts, its size coming back as the pointer does. A
  maximized one comes out floating under the pointer in one motion, the
  pointer holding the same spot of its title bar.
- **Docking.** A window lands by where its middle is, not the pointer.
  - Over the right quarter while the column is empty, a box shows Dock.
  - From outside the column, over a row: its top third places it above,
    its middle third replaces it, its bottom third places it below; a full
    column only replaces. A replaced window is minimized and comes back to
    its row while the column has room.
  - A docked window dragged along the column keeps its row's size; its row
    stays empty and the others step aside live as they will stand (Move
    Here). Its middle beside the column, it comes out floating.
  - While a box shows, the window keeps its size and stops at the area's
    edges; a landing changes only once the middle is 12 px past the line,
    so it does not flicker on it. Escape cancels.
- **Sizes.** A floating window's edges and corners size it, at least
  320 x 240 px; the divider before the column sizes the column, at least
  320 px, leaving the conversation at least 420 px; the gap between rows
  sizes them, its grip showing only under the pointer. While any of these
  drags, or the browser window is resized, nothing animates, so the windows
  follow at once.
- **Motion.** A window that opens or comes back from the taskbar grows from
  96% as it fades in, where it stands, as a macOS window opens; one that
  closes or is minimized shrinks to 96% as it fades out, where it stands.
  The page's first layout and another conversation's show at once.
- **Escape** puts away the window in front while it floats, is anchored or
  is maximized, unless a field holds the keys: one that shows only what it
  always shows, such as Files, closes; one that holds tabs of the user's own,
  such as pages or devices, is minimized. A docked window stays.
- **A window closed opens again where it stood.** Minimizing, restoring and
  maximizing each come back to the place before.

### The taskbar

- **Left: +, then pins, a divider, then open windows.** + lists the kinds
  but Attachments, which opens only from an attachment: it opens a kind's
  window, or a new tab of the browser's. A pinned button keeps its place
  after +, dimmed while its window is closed. Buttons that do not fit go
  into More.
  Dragging a button along its group, the pins or the open windows, arranges
  them.
- **Right: the system area,** after a divider: Agents, Running and Services,
  each with its count, and Schedules. A system button with nothing to show
  is not there. System buttons are not pinnable.
- **A button** shows its tab's own icon, such as the page's or the file's,
  its tab's name and its count of tabs. A click shows the window, or
  minimizes it while it is in front. Resting the pointer on it lists the
  window's tabs, to pick or close one; on a system button the list shows at
  once, with no tooltip, and opens leftward. One list shows at a time. Its
  menu pins, unpins and closes.
- Buttons grow and shrink in width as they come and go.

### The composer

The composer is one rounded box of two rows: the text, and under it Attach,
the workflow and Saved on the left, the model, the context and Send on the
right. The capsule shape goes. Floating windows stay above it. Scroll to
Bottom stands in the middle above it; when the composer moves or changes
size, the conversation follows with motion.

### What is kept

- **A conversation keeps its windows**: each window's place, its floating box
  as shares of the area's width and height, the column's share and its rows',
  the order in front, and where each closed window stood. The page writes them
  when a move, an open or a close ends, never while a drag goes on. Another
  device opening the conversation shows the same layout scaled to its screen;
  a device showing the conversation keeps its own until it opens it again.
- **The user keeps their pins** and **the file tree's choice**, shown or hidden
  and its width, for every conversation and device.
- The file tree hides by itself while showing it would leave the file less
  than 400 px; the user's toggle still shows it then, beside the file. Only
  below 240 px for the file does the tree show over the file instead.

### A phone

A phone shows one window at a time, over the conversation under its top bar,
the window's title bar holding Back. The top bar's Tabs button lists the
taskbar's buttons. A phone never writes the layout.

### The design to build from

`web-ui/windows`:

- `stage.ts`: `StageState`, `useWindowStage(saved?)`, `openingBox`,
  `dropTarget`, `columnSlot`, `arrange`, `arrangeColumn`, and the boxes in
  shares (`WindowBox`).
- `WindowStage.vue`, `WorkWindow.vue`, `Taskbar.vue`: the area, a window's
  chrome, the taskbar.
- `work-windows.ts`: which window each tab of the conversation opens in, from
  its kind's `window`, and each window's tabs and selection.
- `taskbar-icon.ts`: a button's and a tab's icon; `taskbar-menu.ts`: which
  button's list shows.

The plugin SDK's `PanelKind` gains `window: PanelWindow`, with
`interface PanelWindow { id: string; name(): string; icon: Component; openable?: boolean }`,
shared by the kinds of one window, `openable: false` keeping it out of +,
and `start?(): Data`, the data of a tab + opens, which the browser's kind
has. A pinned kind is its window's one tab.

`ChatSession` holds the stage and the taskbar around its transcript and
composer. The host gives it the conversation's tabs and kinds, as
`WorkPanel`'s props were, and keeps the layout. `WorkPanel`,
`SessionOverlay`, `DockWindowArea`, `dock-window.ts`, `AgentsChip` and
`TerminalChip` go; Agents and Running keep their views, without their
overlays.

Stored:

- The panel's document (`conversation_panels`) gains `windows`, the layout as
  JSON the backend checks for size alone (at most 16 KiB), and the change
  `{ op: "windows", windows }` replaces it. A document without it shows no
  window open.
- `Preferences` gains `taskbarPins: string[]` (window ids) and
  `fileTree: { shown: bool, width: u16 }`.

An older page ignores `windows`; a newer page meets an older document as a
conversation with no window open.

## Spike

The prototype in the gallery (`/taskbar-frame`) ran every interaction above
with the product's own views in the windows: Files, Changes, the browser,
Attachments, Agents and Running. It showed that a docked browser and a
floating file read side by side at 1280 x 720, and that boxes kept as shares
fit 1280 x 720, 1440 x 900 and 1920 x 1080.

## Prior art

- [dockview](https://github.com/mathuo/dockview) (`5b9bed8`): its drop target
  divides a group into edges and a center (`calculateQuadrantAsPercentage` in
  `packages/dockview-core/src/dnd/droptarget.ts`, 20% edges) and floats
  groups over the grid. Demi keeps one column of rows, so a drop over a row
  has three bands and no left or right.
- Windows 11's taskbar and Snap: pins, a button per open app, a preview of
  its windows on hover, a window dragged to an edge snaps. Demi's buttons
  keep the tab's name, which Windows' icon-only buttons hide.
- JetBrains tool windows: docked, floating or maximized, the same view in
  each.

## Alternatives

- Keep the right panel and let it split: one more mode for the panel, and the
  dock's windows keep their own rules.
- A window per tab: twenty tabs make twenty windows and twenty buttons.
- Keep the layout in the browser: another device shows the conversation
  without what Alex arranged.

## Open questions

None. Later work, not decided here:

- The branch beside the worktree in the top bar.
- A tab dragged out of a window into a window of its own.
- A tab per file in Files, which needs a file's place kept in a tab that is
  not pinned; Files keeps one view for now.
