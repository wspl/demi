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
`SessionComposer` owns the shared input surface used by the gallery, prototype
and `AgentMessageInput`; each caller supplies its own state and editor behavior.
The application assembles `AgentMessageList`, `SessionSurface`,
`SessionDock` and the shared dialog, menu and form controls. Application CSS owns
page and settings layout only; component appearance belongs to `web-ui`.
Menus use `MenuGroup` for section headings and `MenuItem` for rows. Its `value`
field owns right-aligned metadata; its indicator owns status-dot size and color.
The model menu, host menu, project picker and gallery share these row contracts.

`main.ts` composes the app, router and stores, and advances a deterministic
scripted response clock. The fixture provider's model names are illustrative. Both seeded devices start
online so main-host switching is available immediately.
Conversation content and resource edits live in memory and reset on reload.
Files stay in the browser: image previews use local object URLs, and file
placement is simulated. No credential fields send or persist secrets.

## Prototype behavior under review

Zan opens `demi`, then creates a conversation and sends a message. The local
clock streams a scripted answer. A second message queues behind it. Selecting
another conversation in the sidebar leaves the simulation running; archiving
is refused while a turn is running. Restore brings an archived conversation back into the list.

The project picker changes the conversation's working environment and sidebar
group together, and refuses changes while running. A project cannot be removed
while any conversation still belongs to it. The Cloud option creates only a
simulated project. These behaviors exercise the current baseline for review;
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

Progress and remaining M13.1 coverage are recorded in `progress.md`.

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
and switches directly when selected. The bottom action opens a simulated file browser
with parent navigation, filtering, file previews and folder selection. Its tooltip
shows the full path. The current branch appears as plain text beside its icon,
without a menu, switching or creation actions. Workspace switching is disabled during a running turn or for an archived conversation, while file
browsing remains available. Files and branches are fixtures; no real filesystem,
Git repository or device is modified. Conversations without a device binding display Cloud, with only the cloud
icon and attachment count in the header.
Project groups retain the project list's order regardless
of conversation creation or activity. Sidebar rows
offer pin and archive on hover, plus the shared context menu for rename, move
and multi-selection actions. Project headers right-align device metadata and
replace it with the new-conversation button on hover using a crossfade. The left folder icon stays visible
while hovering and reflects the collapsed/expanded state. A chevron immediately
after the project name fades in on hover over 150ms and rotates with the fold
state; idle conversations have no status glyph.
Cloud workspaces use a cloud icon, while device workspaces show a computer icon
and hostname, centered beside the workspace name. The sidebar has no prototype
caption. Menus, dialogs, settings and the entry page omit instructional and
prototype commentary. Simulation details belong in documentation. Settings expose
Account, Devices, Providers and Usage. The archive button sits immediately to the
right of the conversation title.
The composer and archived-conversation bar transition in both directions and
respect reduced-motion preferences. Shared typography uses macOS grayscale
antialiasing with normal-weight interface text.
