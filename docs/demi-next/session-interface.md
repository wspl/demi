# Session interface

`web-ui` owns the conversation's reusable layout and interaction. The product
supplies backend state and handlers; the gallery supplies fixtures to the same
components. [Web application](web-application.md) owns state and transport,
[Control layout](../control-layout.md) owns common visual rules, and
[Gallery synchronization](web-gallery-sync.md) owns coverage.

## Frame and navigation

The application navigates conversations through the sidebar and URL, without
conversation tabs. `SidebarLayout` arranges the sidebar, session, and optional
right work panel. Each side pane has an independent divider, width bounds, and
open state. The host persists widths. Below the medium breakpoint the side panes
become overlays: opening one closes the other, and the scrim closes the open pane.

The session header shows the title, execution device/status, and workspace.
Metadata moves below the title on narrow screens. Device and workspace are
independent controls; their interaction belongs to [Host menu](host-menu.md).
Git branch is not shown. Interface labels are not selectable; message content,
paths, and editable fields remain selectable.

`ChatSession` owns the transcript, dock, and inspection windows.
`SessionComposer` supplies the shared input surface, also used by
`AgentMessageInput`; callers provide state and editing handlers. The composer
appears during restoration. With no usable model it shows `SessionNoticeBar`
with “No models available.” and a permitted Configure models action. An unusable
previous selection keeps its name and warning and blocks Send. Archived sessions
show the same bar with a Restore conversation action.

## Loading and failure

A session has one load phase: `loading`, `ready`, `reconnecting`, or `failed`.
`SessionStatus` and `sessionPaneStatus` distinguish pending history from empty
history. They must not present a pending or failed read as a new conversation.

| Condition | Presentation |
|---|---|
| Initial history pending | Centered loading pane |
| History ready, models still loading | Readable transcript and model loading state |
| Agent handshake or reconnect pending | Retain history; Connecting in the activity slot |
| Failure with history | Retain history and show a failure notice with Retry |
| Failure without history | Failed pane with Retry |
| Ready with no messages | Empty conversation with its composer |
| Unknown ID after sidebar list is ready | Conversation not found |

Sidebar lists use `loading`, `ready`, or `failed`. An unknown route waits for
list restoration before reporting missing. Retry reloads the affected data;
model catalog failure has its own retry beside the selector rather than hiding
history. [Web application](web-application.md#state-and-connections) defines cache
retention and invalidation.

## Transcript and activity

`AgentMessageList` is the only transcript assembly used by product and gallery.
Thinking, tool, and error blocks use shared `FunctionalBlock` renderers. Thinking
Markdown is visually secondary: 13px text, 20px line height, subtle foreground.
Shell command and output share a rounded bordered base-surface panel with 12px
monospace text and 20px line height. Commands use subtle foreground; output uses
muted foreground while retaining ANSI colors. History starts folded; preview
hosts may supply an initial `open` state. A specimen frame supplies the outer
inset, so its thinking block must not add a second horizontal inset.

Only final text gets the assistant footer: text directly before the next visible
user message, or the idle transcript's last text block. Intermediate updates have
no footer. The footer copies Markdown, shows relative time with an absolute-time
tooltip, and emits a Fork request when enabled. `RelativeTime` shares a one-second
clock and stops it when its last consumer unmounts. Product operations are defined
in [Web application](web-application.md#drafts-sending-and-recovery).

`ActivitySlot` is the transcript's waiting row, rendered only by
`AgentMessageList`. `activitySlotKind` chooses its face in this priority order:

1. Reconnecting shows Connecting.
2. A pending resume shows Retrying if the transcript ends in an error, otherwise
   Resuming. The pending recovery hides that error record. The next server phase,
   a refused request, or connection release clears the pending action.
3. A running turn without active thinking, text, or an executing tool shows
   Requesting after a waiting boundary, such as a user/steer message, compaction,
   abort, or a completed tool. Queue and unconfirmed-submission rows do not
   determine the transcript's active output.
4. Other states have no waiting row.

The row uses the same 28px face as thinking and tool rows. Changing the reason
rolls its label beneath a standing activity mark. When thinking or a tool arrives
while the slot is visible, `useActivityHandoff` temporarily holds the new block
out of the list and rolls the slot to that block's icon and label. After
`ACTIVITY_HANDOFF_MS` (the 450ms face roll plus 80ms), the actual transcript row
occupies the same position. Thinking uses the brain mark; shell uses the terminal
mark and command title; a generic tool uses its name without an icon.

The hold ends if the block leaves the tail, the conversation changes, or the
component unmounts. Text appears immediately, and a block arriving without an
activity slot is appended directly. Resume and Retry therefore share one flow:

```text
Resume -> Resuming -> Requesting -> Thinking or tool output
Retry  -> Retrying -> Requesting -> Thinking or tool output
```

New thinking, tool, and abort rows enter a live transcript with a 200ms fade and
8px slide from the left. `useChromeEntrance` treats loaded history, including the
batch that ends loading, as settled. A row enters once; virtualization remounts
do not animate it again. A row delivered by activity handoff is already settled.
Text streams without this entrance. Reduced motion skips entrance transitions.

## Dock and scrolling

`SessionDock` reserves a permanent 28px control row with an 8px bottom gap above
the composer. Task chips and a fixed 28px Scroll to bottom slot align left.
The arrow uses the opaque solid `IconButton` variant. Hiding it changes opacity
without removing the slot, moving the composer, or changing the scroll range.

`SessionSurface` includes this row and gap in its measured dock height;
`AgentMessageList` uses that measurement for bottom padding. Composer or task
control growth keeps a reader already at the bottom anchored there. A reader
browsing earlier content retains control of their position.

## Agent and terminal inspection

The Agents chip counts only running children and disappears at zero. It toggles
`SubagentPanel`; Running toggles `TerminalPanel`. The windows occupy the same
lower-half session slot above the dock. Opening one closes the other and
crossfades in place. A second click on the same chip folds its window.

The child window shows running children as tabs with a robot mark and status.
History opens a searchable menu of finished children; selecting one adds that
child beside the running tabs. Inspection shows its transcript without a composer.
Stop actions call the supplied agent-protocol handler.

The terminal window shows live jobs as tabs with a terminal mark and status. An
exited-job inspect shows that job alone. Its xterm is read-only. The 16 ANSI colors
use a fixed light/dark palette; background, cursor, and selection follow the live
surface and accent so selection remains visible without focus.

`SessionOverlay` owns the shared window edge, dismissal, and motion: one overlay
stroke and light shadow, with a 150ms opacity/scale transition from the dock edge.
Reduced motion skips the transition. An outside pointer closes the window and
is not passed to underlying controls. Floating menus and the two dock toggles
are excluded because they handle their own interaction.

## Tabs and work panel

Every tab row uses `TabStrip` with `TabItem` children. Tabs share a width. A short
separator in each gap fades when either adjacent tab is active or hovered.
Overflow scrolls horizontally without a scrollbar, shows an edge fade where more
content exists, and reveals the active tab without scrolling the page. Insertions
grow and removals collapse from their current width while fading; reduced motion
skips the transition. Strip observers are released on unmount.

The strip receives its surface once: on a base surface, active tabs use the
regular surface; on a raised work panel, they use the base surface. Tabs, hover
fills, close controls, and fades derive colors from that strip.

The work panel continues the session sheet behind a hairline resize divider.
Its tab row aligns with the session header. The session-header open control is
visible only while the panel is closed; the panel has its own fold control.
The host retains file/diff tabs per conversation and their active ID. File tabs
show a file icon; diff tabs add a compare mark; tooltips show full paths. Closing
an active tab selects the preceding tab, or the next remaining tab if it was first.

The shared frame and `WorkPanel` exist, with assembled and standalone Session
gallery examples. Content is a placeholder. File/diff viewers, actions that open
them, and product mounting are deferred; a gallery panel is not evidence of those
completed flows.

## Settings and overlays

Settings uses the shared shell and page components over presentation models.
The enabled sections and permissions are defined in [Product](product.md).
A whole deferred page is disabled on the rail. Deferred rows on a usable page
stay visible with an In development tooltip; their wrapper retains pointer events
so that explanation remains available.

A dialog opened from settings stacks above it. Escape and scrim close only the
top dialog; closing the shell closes its stack. Dialog instances stay mounted and
use open state so leave transitions run. The workspace dialog resets its form
when reopened. Device pairing can open above that form or from the application
root, using the same shared dialog.

The account header and rail show the display name without secondary text; the
account menu may show email. Product appearance and control sizes come from
[Control layout](../control-layout.md). Simulation details and prototype
instructions belong in documentation, not the product interface.
