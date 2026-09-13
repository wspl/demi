# Sidebar ordering and interaction

`web-ui/sidebar/AppSidebar` owns sidebar presentation, selection, folding, and
reorder interaction. The host supplies ordered projects and conversations and
handles mutations. Product state persists saved-row changes through the backend;
gallery state applies them to fixture arrays. Activity never changes manual order.

## Order and persistence

Projects follow their supplied array. Each project keeps conversation array order
within two partitions: pinned first, then unpinned. Conversations without a project
form their own group. Empty projects remain visible. New conversations enter the
front of the unpinned partition; new projects append.

The backend stores sort positions in the control database. Renaming, inference,
model changes, archive/restore, and target changes retain those positions. Pinning
moves a conversation to the other partition while retaining its sort position.
If positions tie after grouping changes, conversation ID gives a stable tie-break.
The browser consumes the resulting ordered snapshots rather than sorting by
updated timestamps or recent-directory history.

A reorder is an insertion: `{ kind, id, beforeId }`, where null means append.
The shared component uses `kind: project | conversation`; the product maps
`project` to the API's `workspace`. `POST /api/sidebar/reorder` writes the affected
partition atomically. It returns 204 on success; the product refreshes account
state to obtain the new order. Invalid ownership, group, pin partition, or archived
conversation targets are rejected with `invalid_order`.

Saved conversation and project ordering is wired to that API. A failed request
shows an error and retains the last confirmed order. An unsent local conversation
is reordered locally; it has no backend row yet. The product must not send its
local UUID as an insertion target for a saved-row operation.

## Drag and keyboard reorder

```text
Pointer down on a row title
    |
    +-- move less than 5px --> ordinary click
    |
    +-- move at least 5px --> floating title and insertion line
                               |
                               +-- valid drop --> emit insertion request
                               |                    |
                               |                    +-- host updates ordered data
                               |
                               +-- Escape, cancel, blur, invalid drop --> unchanged
```

Dragging keeps the actual order intact until drop. It moves one row regardless of
multi-selection. A conversation can move only within its current project and pin
partition; reordering cannot change its execution target. Use Move to project or
[host controls](host-menu.md) for that operation. Project headers reorder among
projects. `reorderPeers` and `sidebarDrop` resolve these boundaries and no-op drops.

The pointer's upper or lower half of a valid row chooses insertion before or
after it. Both representations of the same gap draw the same insertion line.
Near the scroll region's edge, the list scrolls and recomputes the target every
animation frame. Pin, archive, create, and rename controls do not start dragging.
A completed drag suppresses its synthetic click so it does not also open or fold
a row. Cancellation and unmount release the animation frame and window listeners.

Dragging a project temporarily folds every project without changing saved fold
preferences. The dragged header stays visible while the list contracts. Drop or
cancellation restores the previous folds, waits for layout motion, then scrolls
the source project toward the viewport's vertical center. Scroll limits clamp it
near list ends. Reduced motion uses instant positioning.

Alt+Up or Alt+Down reorders the keyboard-focused row within the same permitted
partition. At a partition boundary it does nothing. `AppSidebar` emits requests;
`useSidebarDrag` owns pointer state, feedback, cancellation, and edge scrolling.
The shared `moveBefore` utility provides array insertion where a host needs it.

## Selection and row actions

A plain conversation click selects and opens it. Ctrl/Command-click toggles a
selection; Shift-click selects a range of visible conversation rows. Arrow keys
move focus and selection; Shift+arrows extend the range. Enter opens a conversation
or folds a project; Left/Right folds or unfolds projects. Space toggles the focused
conversation, Ctrl/Command+A selects visible conversations, and Escape collapses
the selection to the open conversation. Selection is shared across groups.

A context menu on a selected conversation acts on the selection; opening it on
another row first selects that row. F2 starts rename. Enter or blur commits a
nonempty trimmed title once; Escape cancels. Ctrl/Command+Shift+P toggles pinning.
The product hides deletion and exposes archive instead. Archive is disabled for
an active conversation. Batch mutations preserve successful rows and report
failures as defined in [Web API](web-api.md).

Rows keep normal font weight. The open row remains distinguishable within a
larger selection. A running root or child produces the active dot; unread finished
results use green, while unread errors or aborts use orange. Settled rows have no
dot. Long titles begin a constant-speed marquee after 500ms hover when clipped;
leaving stops it, and unmount clears its timer. Reduced motion keeps titles still.

Conversation actions use 24px targets, a 2px gap, and a 2px right inset: Archive,
then Pin at the right edge. A pinned glyph occupies that same pin slot while idle.
Project headers keep their folder icon visible, crossfade right-hand device
metadata to New conversation on hover, and reveal a rotating fold chevron beside
the name over 150ms. Cloud projects show a Cloud icon; user-device projects show
the host name. New conversation and Add project also appear on section headings.

## Layout motion and scrolling

Headings, project headers, and visible conversations use one keyed
`TransitionGroup`. Insertions grow in flow. Removed or folded rows shrink their
height, gap, and margin while fading, so list height and scroll position follow
smoothly. Remaining entries move over 180ms. Reduced motion disables transitions.
Section headings stick to the top; project headers stick below the Projects heading.

The sidebar uses a native scroll region with `scrollbar-gutter: stable both-edges`.
Every sidebar column reserves symmetric edge space so labels and row backgrounds
remain aligned whether the list overflows. This is distinct from the overlay
thumb used by general `ScrollArea` surfaces in [Control layout](../control-layout.md).

## Acceptance

Verify saved conversation and project order after refresh and backend restart;
activity independence; pin/group boundaries; no-op and invalid drops; local drafts
next to saved rows; Alt+Up/Down; long-list edge scrolling; project fold restoration;
Escape, pointer cancellation, and window blur; and reduced motion. Tests cover
shared grouping/reorder helpers, backend sidebar transactions, and browser-store
integration. Gallery fixtures exercise the shared interaction, while persistence
and partial backend failures require the product adapter.
