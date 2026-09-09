# Web prototype

The application prototype lives in `packages/web`. Run `bun run web:dev` and
open `http://127.0.0.1:18934`; `bun run web:build` writes `packages/web/dist`.
No backend, runner, credentials or model service is needed.

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
after refresh or route changes, allowing initial layout changes to settle. This is
opt-in preview navigation; conversation scrolling retains its existing behavior.
`SessionComposer` owns the shared input surface used by the gallery, prototype
and `AgentMessageInput`; each caller supplies its own state and editor behavior.
The application assembles `AgentMessageList`, `SessionSurface`,
`SessionDock` and the shared dialog, menu and form controls. Application CSS owns
page and settings layout only; component appearance belongs to `web-ui`.
Menus use `MenuGroup` for section headings and `MenuItem` for rows. Its `value`
field owns right-aligned metadata; its indicator owns status-dot size and color.
The model menu, host menu, project picker and gallery share these row contracts.
Floating menus use intrinsic content width with a 160px minimum (192px with a
search field), capped at 384px and the viewport width minus 32px. Labels truncate
at the cap. Shortcut space exists only for actual shortcuts. The virtualized
conversation history uses 320px for titles and timestamps; embedded file and
target lists fill their containing panel. Ordinary menus do not set fixed widths.

`main.ts` composes the app, router and stores, and advances a deterministic
scripted response clock. The fixture provider's model names are illustrative. Both seeded devices start
online so main-host switching is available immediately.
Conversation content and resource edits live in memory and reset on reload.
Files stay in the browser: image previews use local object URLs, and file
placement is simulated. The composer's Add menu offers local files and, when the
conversation has a host, a remote file: the shared browser in file mode over the
main and attached hosts, whose chosen path joins the draft as a reference. No credential fields send or persist secrets.

## Prototype behavior under review

Zan opens `demi`, then creates a conversation and sends a message. The local
clock streams a scripted answer. A second message queues behind it. Selecting
another conversation in the sidebar leaves the simulation running; archiving
is refused while a turn is running. The sidebar's Skills and Archived entries open
those settings sections; the Archived page is a searchable list whose Restore brings
a conversation back into the sidebar and opens it.

The project picker changes the conversation's working environment and sidebar
group together, and refuses changes while running. A project cannot be removed
while any conversation still belongs to it. The Cloud option creates a simulated directory/project on the user's one
Cloud device; creating a second project reuses that identity. These behaviors exercise the current baseline for review;
prototype additions are decided in M13.2.

## Verification

- `packages/web/src/conversation/store.test.ts`: scripted turn completion, queue
  order, cancellation and recovery, target/archive guards and file-only input.
- `packages/core/src/__tests__/platform-entrypoints.test.ts`: workspace and browser
  package boundary enforcement.
- Existing `packages/web-ui` tests: shared rendering and interaction contracts.
- Browser walkthrough: create/switch conversations, streaming/queue/stop,
  failure/Retry, archive/restore, project creation/switch, settings, light/dark,
  mobile sidebar, attachment preview, and unknown-conversation navigation.
- `bun run typecheck:web` and `bun run web:build`: browser compilation.

Prototype acceptance follows the Web delivery stages in `roadmap.md`; verified
checks are recorded in `progress.md`.

Navigation uses the sidebar and URL only, with no conversation tabs. Interface
labels and controls are not selectable; message content, paths and editable
fields remain selectable. Selected sidebar conversations retain normal weight.

Conversation headers show the title with device name/status, workspace name and
branch to its right. Metadata moves below the title on narrow screens. Hosts,
and workspace are independent controls; the branch is read-only metadata. The host menu separates the main
execution environment from named attached devices, and offers attach,
detach and device registration entry points (see `host-menu.md`). Main-host
selection opens a directory chooser on the selected device; choosing a folder moves only this conversation
to the matching workspace, creating its prototype record when needed. Offline
devices can be attached but cannot be selected as the main environment. The workspace control
opens up to eight recent directories on the current device, marks the current one,
and switches directly when selected. The bottom action opens the shared file browser
(`docs/file-browser.md`) in folder mode over the device's fixture tree, with the
device's workspaces as places and the other devices in its sidebar; choosing a
folder moves the conversation. The new-project form's Browse button opens the same
browser for the chosen device. Its tooltip shows the full path. The current branch appears as plain text beside its icon,
without a menu, switching or creation actions. Workspace switching is disabled during a running turn or for an archived conversation, while file
browsing remains available. Files and branches are fixtures; no real filesystem,
Git repository or device is modified. Conversations selecting Cloud display its icon and attachment count even before
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

The settings dialog is the shared shell with the product rail
(`web-ui/settings/sections.ts`): General, Account, Notifications; Models &
providers, MCP servers, Skills; Devices, Archived, Keyboard, Data & privacy. Every page is a `web-ui` component over a presentation model;
`web/settings/SettingsDialog.vue` supplies the prototype's state
(`prototype/settings.ts`) and decides what a row does to the app. General writes the
theme choice, tone, accent and transcript text size onto the document as they
change. Account runs the change-email and change-password flows with a timer in
place of the server (`wrong` is the one password refused, `000000` the one code).
The providers page edits the same provider entries the composer reads: a provider
is offered in the model menu while it is enabled and its last check passed, and only
its enabled models are listed. Keyboard rebinds the shortcuts `App` listens for,
through the one notation `web-ui/ui/shortcut.ts` defines; a binding another action
holds is refused on the page's note. Data & privacy's Delete all empties the
conversation list in place. A dialog
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
only maps the prototype's projects and devices onto it. Connect new device, from
the host menu, opens the Add device pairing dialog at the app root; the Devices
settings page runs the same claim.
The composer appears immediately on conversation changes and restoration.
Shared typography uses macOS grayscale
antialiasing with normal-weight interface text.

Sidebar ordering and motion follow `sidebar-order.md`. Projects and conversations
can be reordered by dragging or Alt+Up/Down. The fixture contains 40 conversations
to exercise scrolling and animation. Ordering follows the supplied arrays, with
pinned conversations first within each project; activity does not reorder rows.

Seeded conversations include multi-turn text, tables, checklists, code, thinking,
completed and failed shell outputs, context summaries, and recoverable error/abort
states. `prototype/transcripts.ts` owns these typed block fixtures. New conversations
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

Every control comes in height families that line up with each other: 28px
(`Button` md, `IconButton` md, `TextInput`, `Segmented` md, `Dropdown` md), 24px
(`sm` of each) and 20px (`xs`). A surface picks one family per kind of control and
keeps to it: all of its text inputs one height, all of its buttons another. In
settings cards text inputs are 28px, since a line of text wants that room, and
buttons, icon buttons, segmented and dropdown controls are 24px, since a bordered
28px icon button reads heavy in a row. Two buttons of different heights in one
card, or two inputs, is a mistake. Chrome outside the cards (a dialog's search,
the narrow back row) stays at 28px, and a rail caption's action or a hover action
inside a 32px list row uses 20px. A bare input (no frame in any state) is the
exception: its hit area stretches to the row's content box, 28px in a compact row
and 32px in a regular one, so the value is easy to click into.

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
unassigned conversations use directories on that device. The prototype's Cloud,
project list and environment selector already express this structure. Integrating
the backend does not require separate machine-ownership controls for projects or
conversations. Authoritative target and persistence rules live in
[sessions-and-targets.md](sessions-and-targets.md) and
[managed-hosts.md](managed-hosts.md); implementation acceptance is recorded in
[progress.md](progress.md).

`web-ui/cloud/CloudSettings.vue` owns the shared Cloud status, storage-limit
readout, reset confirmation, progress and retry interaction. `SettingsDevices`
mounts it in both the product prototype and the gallery. The component emits an
operation ID; a failed reset retries with that same ID. Its confirmation explains
that reset stops all of the user's Cloud tasks, replaces system packages and
settings, and retains home files.

`web/prototype/cloud.ts` supplies simulated phases and stops prototype Cloud
streams when reset begins. Gallery fixtures supply the same presentation
contract. Neither currently calls the authenticated backend endpoints
`GET /api/cloud` and `POST /api/cloud/reset`. Simulated completion verifies the
presentation flow; it does not verify disk persistence or backend recovery.

The remaining product integration maps the authenticated user's Cloud status to
the shared component, submits its operation ID and reads the operation result.
Request acceptance is distinct from completion. Loading, unavailable Cloud,
startup failure and reset failure need observable feedback; the interface does
not need VM, disk-generation or runner controls. Backend wake is automatic when
an operation needs the machine, so sleeping must not require manual power-on.

Integration checks should cover sleeping, starting, ready, resetting and failed
states, including a guest that cannot connect, with consistent product/gallery
presentation. Backend persistence and reset acceptance belong to the scenarios
recorded in `progress.md`, rather than to the browser simulation.

The prototype currently includes a fixture Cloud file browser and project move
controls. Backend support for those operations does not decide which entry points
the product offers. Product decisions about exposing Cloud browsing or moving a
conversation out of a project remain separate from this implementation record.
