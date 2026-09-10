# Web application

The backend-integrated application lives in `packages/web`. Run the backend,
then `bun run web:dev` at `http://127.0.0.1:18934`; `bun run web:build` writes
`packages/web/dist`. The gallery remains independent of credentials and model
services. State ownership and API behavior are documented in
[web integration](../web-integration.md).

## Technology and ownership

Vue 3 and TypeScript render the interface; Vite runs and builds it. vue-router
owns navigation and Pinia owns application state. Tailwind 4 compiles the shared
web-ui theme and components. `docs/package-boundaries.md` defines module ownership.
The stack continues the existing browser packages and the product design.

Gallery and application import sidebar presentation from `web-ui/sidebar`.
The gallery's FunctionalBlock specimens use the same `ThinkingBlock`, `ToolShellBlock`,
and `ErrorBlock` as the product transcript, including thinking Markdown and live labels.
Thinking bodies use 13px text, 20px line height, and the subtle foreground token
to keep them visually secondary to the response. Shell commands and output share
a rounded, subtly bordered base-surface panel with 12px monospace text and 20px
line height. Commands use the subtle foreground and output uses the muted
foreground, while ANSI colors remain available for terminal emphasis.
Their optional `open` models let previews set the initial state; product history
still starts folded. The specimen frame owns the outer inset, so thinking blocks
use a zero `--agent-pad-x` inside it to avoid doubling the padding.
Gallery catalog pages remember their scroll position per route in session storage.
The shared `useSavedScroll` composable restores the visible section and its offset
after refresh or route changes, allowing initial layout changes to settle. Every
catalog page has a top-bar view (`?view=`). Long pages split: Session (Tab bar,
Composer, Blocks, Turns, States, Session, Windows), Primitives (Buttons, Fields, Marks), Overlays
(Menus, Dialogs), Files (Browser, Dialogs), Settings (Settings, Account, Device,
Sign in). A short page has one view named after the page. The default view omits
the query. Session defaults to the assembled Session view, which fills the pane.
The Session Tab bar specimen closes a tab the way Chrome does: the departing
tab keeps its place, fades, and collapses from its current width so the
remaining tabs slide in; reduced motion skips the transition.
Appearance (paradigm, mode, tone, accent, density, radius, shadow) opens from a
palette in the top-right as a floating menu. Each view keeps its own scroll. This is opt-in preview navigation;
conversation scrolling retains its existing behavior.
`SessionComposer` owns the shared input surface used by the gallery, product
and `AgentMessageInput`; each caller supplies its own state and editor behavior.
The application supplies state and handlers to `ChatSession`, `SidebarLayout`,
`WorkspaceDirectoryMenu`, and the shared dialog, menu and form controls. Their
interaction and layout belong to `web-ui`.
The session dock's Agents chip opens `SubagentPanel` the same way Running opens
the terminal window: a second click on the chip closes it. The window sits over
the lower half of the session, above the dock chips: a tab bar with a robot mark
and status, a History control of completed children, a fold control, the
child's transcript, and no composer. The tab bar lists every running child; the
History control opens a searchable menu of finished children, and choosing one
shows that child beside the running tabs. Its edge is `overlay-window`: one
`--line-overlay` stroke and a light drop, not a menu hairline and not a dialog
scrim. Open and close use the same 150ms opacity and scale as menus and
dialogs, from the dock edge; reduced motion skips the transition. A pointer
outside the window closes it and is not delivered to what is underneath; a
click on a floating menu, or on the Running or Agents chip, does not close it
that way — those chips toggle the window themselves.
The dock's Running chip opens the same window for live shell jobs: tabs use a
terminal mark and status, and the body is a read-only xterm that reveals the
job's output. The 16 ANSI colors are a fixed light/dark palette; the ground,
cursor and selection follow the live surface and accent tokens so a read-only
selection stays as visible as a focused one. A running inspect lists every
running job; an exited inspect is that job only. Opening one window closes the
other; they occupy the same half-session slot, so a switch crossfades in place
instead of sharing the height. The Session page's Windows view exhibits both
windows on their own, without the session behind them.
Menus use `MenuGroup` for section headings and `MenuItem` for rows. Its `value`
field owns right-aligned metadata in the subtle foreground, 16px from the
label; its indicator owns status-dot size and color.
The model menu, host menu, project picker and gallery share these row contracts.
Floating menus use intrinsic content width with a 160px minimum (192px with a
search field), capped at 384px and the viewport width minus 32px. Labels truncate
at the cap. Shortcut space exists only for actual shortcuts. The virtualized
conversation history uses 320px for titles and timestamps; embedded file and
target lists fill their containing panel. Ordinary menus do not set fixed widths.

`main.ts` composes the app, router and account-scoped stores. REST snapshots
reconcile metadata while the agent WebSocket carries live transcript, queue,
child and terminal updates. Leaving a conversation detaches its client and keeps
the backend task running. Reload restores history and per-user local drafts.

Local attachments report actual transfer progress. A failed upload retains its
tile and offers Retry; Send waits until every file is ready. Model-supported
media go to the model, and other files go to the working directory. Removing a
file cancels its transfer. Remote-file selection uses real main/attached user
devices, retaining device identity and the full path in the tooltip and request.
Credential queries never expose saved secrets.

## Conversation and project behavior

A second message queues behind a running turn. Archive and main-environment
changes are refused while running. Archived conversations can be restored from
the shared settings list. Skills remains disabled.

The project picker changes the conversation's working environment and sidebar
group through backend APIs. A project with conversations cannot be removed.
Cloud projects use different directories on the same persistent user Cloud.
Choosing another directory on a device sets a direct target unless that directory
already identifies a project.

## Verification

- `packages/web/src/conversation/store.test.ts`: HTTP-mocked creation, metadata
  reconciliation, partial batch failures and read acknowledgement.
- `packages/core/src/__tests__/platform-entrypoints.test.ts`: workspace and browser
  package boundary enforcement.
- Existing `packages/web-ui` tests: shared rendering and interaction contracts.
- Browser walkthrough: create/switch conversations, streaming/queue/stop,
  failure/Retry, archive/restore, project creation/switch, settings, light/dark,
  mobile sidebar, attachment upload phases, session load states, and unknown-conversation navigation.
- `bun run typecheck:web` and `bun run web:build`: browser compilation.

Product acceptance follows the Web delivery stages in `roadmap.md`; verified
checks are recorded in `progress.md`.

Navigation uses the sidebar and URL only, with no conversation tabs. Interface
labels and controls are not selectable; message content, paths and editable
fields remain selectable. Selected sidebar conversations retain normal weight.

The session pane has one load phase (`ready | loading | reconnecting | failed`).
Opening or switching to a persisted conversation uses the centered `SessionStatus`
loading pane, including when messages are cached. `web/conversation/store.ts`
keeps that phase through history loading and the initial agent connection handshake.
Archived conversations and conversations without an available model finish loading
after their history arrives. Loading history never reads as an empty conversation.
A dropped socket in an already open session keeps a
cached transcript and shows Connecting as the same tail row as Requesting
(`LoadingBlock`); without a cache it is that row alone, not a centered pane or
a bar. A failed restore offers Retry, which sweeps loading then ready. An
unknown id waits until the sidebar list is ready, then shows Conversation not
found. The sidebar list uses the same three-way status: a spinner while loading,
Retry when restore fails (the same sweep), and a first run only after the list
is ready. `RestoreSweep` times gallery-only simulated restores; real request completion drives the product. `SessionStatus` owns the
copy and chrome for loading, failed, empty, and missing. The Session States and
Sidebar States gallery views pin every kind; failed (and missing) are live.

Conversation headers show the title with device name/status and workspace name
to its right. Metadata moves below the title on narrow screens. Hosts
and workspace are independent controls. Git branch is not shown. The host menu separates the main
execution environment from named attached devices, and offers attach,
detach and device registration entry points (see `host-menu.md`). Main-host
selection opens a directory chooser on the selected device; choosing a folder moves only this conversation
to the matching project or sets a direct device target. Offline
devices can be attached but cannot be selected as the main environment. The workspace control
opens up to eight recent directories on the current device, marks the current one,
and switches directly when selected. The bottom action opens the shared file browser
(`docs/file-browser.md`) in folder mode over the device's actual directory source, with the
device's workspaces as places and the other devices in its sidebar; choosing a
folder moves the conversation. The new-project form's Browse button opens the same
browser for the chosen device. Its tooltip shows the full path. Workspace switching is disabled during a running turn or for an archived conversation, while file
inspection remains available. User-device directory creation and uploads modify that device through its backend adapter. Conversations selecting Cloud display its icon and attachment count even before
allocation; sleeping and starting are lifecycle states of that same selection.
Project groups retain the project list's order regardless
of conversation creation or activity. Sidebar rows
offer pin and archive on hover, plus the shared context menu for rename, move
and multi-selection actions. Project headers right-align device metadata and
replace it with the new-conversation button on hover using a crossfade. The left folder icon stays visible
while hovering and reflects the collapsed/expanded state. A chevron immediately
after the project name fades in on hover over 150ms and rotates with the fold
state; idle conversations have no status glyph.
Cloud workspaces use a cloud icon, while device workspaces show an online/offline status dot
and hostname, centered beside the workspace name. The sidebar has no prototype
caption. Menus, dialogs, settings and the entry page omit instructional and
prototype commentary. Simulation details belong in documentation. The archive button sits immediately to the
right of the conversation title.

## Sign in

`/login` is the shared `web-ui` email-and-password page: the form on the
base surface, a wide empty intro on the session surface. The email field,
password field and Sign in button use the 36px `lg` family. Below a tablet
width the intro hides. There is no registration or password recovery.
`web/auth/LoginPage.vue` submits credentials through `auth/session.ts` to
`POST /api/auth/login`. The backend reports incorrect credentials and rate
limits. Startup waits for `GET /api/auth/me` before mounting the route, and
navigation rechecks an established session. An expired session returns to the
login page; a temporary server failure preserves the identity and reports the
error. Logout calls `POST /api/auth/logout` before loading a fresh document,
which releases account-scoped browser state. See [web authentication](../web-authentication.md)
for the current integration boundary.

## Settings

The sidebar account and the settings rail show the name only; nothing sits
under it. The account menu can still show the email.
The settings dialog is the shared shell with the product rail
(`web-ui/settings/sections.ts`): General, Account, Notifications; Models &
providers, MCP servers, Skills; Devices, Archived, Keyboard, Data & privacy. Every page is a `web-ui` component over a presentation model;
`web/settings/SettingsDialog.vue` supplies backend state and API handlers. Deferred
entries stay visible and disabled, with the shared `In development` tooltip.
A whole unused page is disabled on the rail, not opened as a page of muted
controls: Notifications, MCP servers, Skills and Data & privacy. Skills is also
disabled in the sidebar. Rows on an otherwise usable page stay on that page:
language and delete account. A disabled control keeps pointer events on its
wrapper so the tooltip can show. General writes the
theme choice, tone, accent and transcript text size onto the document as they
change, and persists per-field preferences through the backend. There is no saving indicator. Account runs verified email and password changes through real endpoints.
The providers page edits the same provider entries the composer reads: a provider
is offered in the model menu while it is visible and available in the current environment, and only
its enabled models are listed. Keyboard rebinds the shortcuts `App` listens for,
through the one notation `web-ui/ui/shortcut.ts` defines; a binding another action
holds is refused on the page's note. A dialog
opened from a settings page (a credential flow, adding a server, pairing a device)
stacks on the settings dialog: the shell stays, Escape and the scrim close only the
top, and closing the shell takes the stack with it. Dialogs stay mounted and open
by state (`settingsOpen`, `targetOpen`, a chosen host) rather than by `v-if`, so
closing plays the dialog's leave; the working-environment dialog resets its form on
each opening. That dialog is the shared `hosts/WorkspaceDialog` (project list; a new-project
form that branches on Device or Cloud cards, Device first: a device asks which one,
with Add device beside the menu starting the pairing dialog stacked on the form, and
a directory on it, the project taking the folder's name; the Cloud asks only a name; the
folder browser as a page of it); `web/targets/TargetDialog.vue`
maps backend projects/devices and submits create or move requests. Connect new device, from
the host menu, opens the Add device pairing dialog at the app root; the Devices
settings page runs the same claim.
The composer appears immediately on conversation changes and restoration.
With no usable model, `SessionNoticeBar` replaces the input: No models
available. on the left, Configure models on the right (hidden if the user
cannot open that page). If the last chosen model cannot send, the
chip keeps that name, shows a warning, and send stays blocked until the user
picks another. An archived conversation uses the same bar: This conversation
is archived. on the left, Restore conversation on the right.
Shared typography uses macOS grayscale
antialiasing with normal-weight interface text.

Sidebar ordering and motion follow `sidebar-order.md`. Projects and conversations
can be reordered by dragging or Alt+Up/Down. The gallery fixtures exercise scrolling and animation. Ordering follows the supplied arrays, with
pinned conversations first within each project; activity does not reorder rows.

New conversations
start empty. Sidebar project and conversation action buttons share a 4px right inset
and centered 24px targets; conversation actions have a 2px gap.

## Product appearance

The product uses Regular density, Medium radius and Hairline shadows; those are
fixed. Tone (Ink or Warm), accent, the theme and the transcript text size are the
user's to choose on the General settings page.
`web-ui/theme/productAppearance.ts` defines the fixed axes and the catalogs of
tones and accents; `web-ui/styles/product-appearance.css` owns every token they
need in light and dark. The web composition root applies the axes and the gallery
exposes the same Demi preset, plus further tones for its own paradigms.

## Scroll regions and outlines

A scroll region clips both axes: `overflow-y: auto` makes `overflow-x` non-visible
too, so anything a child draws outside its box, such as a `ring` (a box-shadow), a
focus ring, or a corner badge, is cut off when the child sits flush with the
region's edge. Rules:

- Every scroll region carries horizontal padding of at least the widest outline
  drawn inside it. Where the layout wants the content flush, pair the padding with
  a matching negative margin (`-mx-1 px-1`) so the region grows instead of the
  content shrinking.
- Never fix a clipped outline on the child; fix the region.
- Product scroll regions are `ScrollArea`s: the native bar is hidden and a thumb is
  drawn over the content at the right edge, so the bar takes no room and the
  content keeps symmetric padding whether or not it overflows. Padding for rings
  goes on the viewport (`viewportClass`); the region itself is a `min-h-0` flex
  item. The thumb shows on hover or while scrolling and can be dragged.
- The gallery audits this: `demiAuditClipping()` in the browser console, and
  automatically after each gallery navigation in development, lists every
  outlined element that a scroll region would clip.

## Control size families

Every control comes in height families that line up with each other: 36px
(`Button` lg, `TextInput` lg), 28px (`Button` md, `IconButton` md, `TextInput`
md, `Segmented` md, `Dropdown` md), 24px (`sm` of each) and 20px (`xs`). A
surface picks one family per kind of control and keeps to it: all of its text
inputs one height, all of its buttons another. Isolated page forms (sign-in)
use 36px for both the fields and the submit button. In settings cards text
inputs are 28px, since a line of text wants that room, and buttons, icon
buttons, segmented and dropdown controls are 24px, since a bordered 28px icon
button reads heavy in a row. Two buttons of different heights in one card, or
two inputs, is a mistake. Chrome outside the cards (a dialog's search, the
narrow back row) stays at 28px, and a rail caption's action or a hover action
inside a 32px list row uses 20px. A bare input (no frame in any state) is the
exception: its hit area stretches to the row's content box, 28px in a compact
row and 32px in a regular one, so the value is easy to click into.

The gallery audits this: `demiAuditControlSizes()` in the browser console, and
automatically after each gallery navigation in development, lists every settings
card whose inputs or buttons mix families.

## Colour transitions and `transparent`

Colour properties interpolate with premultiplied alpha in Chromium and WebKit, so a
background, border, text colour, ring, outline, fill or stroke that fades in from
`transparent` keeps its hue throughout; it does not pass through black. Measured in
both engines: mid-transition values keep the target's hue at half alpha for every
property the gallery transitions.

`scrollbar-color` is the exception: Chromium interpolates it without premultiplying,
so a thumb fading in from the `transparent` keyword (black at zero alpha) goes through
dark greys before it turns light in a dark theme. Its hidden state is therefore the
thumb's own hue at zero alpha, written as `rgb(from <colour> r g b / 0)`, never the
bare keyword and never a `color-mix` with 0% of the colour, which also collapses to
transparent black. Any new property found to behave this way gets the same treatment.

## Cloud behavior and backend integration

The current backend gives each user one persistent Cloud device; projects and
unassigned conversations use directories on that device. The product's Cloud,
project list and environment selector express this structure without separate machine-ownership controls. Authoritative target and persistence rules live in
[sessions-and-targets.md](sessions-and-targets.md) and
[managed-hosts.md](managed-hosts.md); implementation acceptance is recorded in
[progress.md](progress.md).

`web-ui/cloud/CloudSettings.vue` owns the shared Cloud status, storage-limit
readout, reset confirmation, progress and retry interaction. `SettingsDevices`
mounts it in both the product and the gallery. The component emits an
operation ID; a failed reset retries with that same ID. Its confirmation explains
that reset stops all of the user's Cloud tasks, replaces system packages and
settings, and retains home files.

`web/settings/DevicesPanel.vue` maps Cloud state from the account snapshot and
submits reset operation IDs to `/api/cloud/reset`. It follows actual operation
phases; request acceptance is distinct from completion. Failed retries reuse
the same ID. The gallery supplies local phases to the same component.

The backend wakes Cloud automatically when an operation needs it. The product
keeps Cloud folder browsing disabled, while workspace uploads and execution use
the existing backend capability. Device browsing remains available. Backend
persistence/reset scenarios are recorded in `progress.md`; browser verification
uses a disposable backend and simulated provisioner.
