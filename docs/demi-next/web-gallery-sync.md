# Web and gallery synchronization

`web` and `web-gallery` consume reusable visual behavior from `web-ui`.
Neither product imports the other. A shared UI change includes both its product
usage and a gallery specimen in the same checkpoint; verification runs all three
browser package typechecks and checks affected specimens in the browser.

## Coverage

| Surface | Shared implementation | Gallery coverage |
| --- | --- | --- |
| Sign in: email and password on the left, a wide empty intro on the right; wrong credentials, lockout, and an ended session | auth/EmailLoginPage | Sign in page over pinned phases; `web` LoginPage over the real authentication adapter |
| Sidebar entries: New, then Skills (disabled, In development tooltip) and Archived opening its settings section; New on the Conversations heading | AppSidebar `openSettings(section)` | Sidebar live specimen; `web` opens Archived from the entry |
| Archived conversations: searchable list with Restore | settings/SettingsArchived | Settings full mock; `web` restores and opens the conversation |
| Marquee, read indicators, ordering, temporary folding and centered scrolling | Sidebar components and drag controller | Sidebar live specimen with mutable data |
| Menu sizing, label/value rows, section headings and status indicators | Menu, MenuItem, MenuGroup | Overlays specimens |
| Host search, Cloud option and bound-device disabling | hosts/HostPicker | Main and attached picker specimens in Overlays |
| Working environment: project list, new-project form, Browse… as a folder browser page | hosts/WorkspaceDialog over `WorkspaceProject` / `WorkspaceDevice` | Files page, pinned on the form and the list; `web` TargetDialog supplies backend projects/devices and handlers |
| Device pairing from anywhere: the host menu's Connect new device | devices/DevicePairingDialog with `useDevicePairing` at the app root | Settings Devices specimen; `web` App hosts the dialog |
| Tab strip: close and insert collapse or grow from the current width | TabStrip, TabItem | Session Tab bar (GalleryTabBar, a fixture bar over the strip: the product has no conversation tabs, and `AgentTabBar` is bound to the legacy workspace store) and overlay inspect tabs |
| Subagent roster and inspect (live count only; no chip when all children finish) | AgentsChip, SubagentPanel over SessionOverlay, SubagentHistoryMenu | Session Windows view and dock chip; `web` ChatPage over conversation.subagents |
| Live shell jobs | TerminalChip, TerminalPanel, XtermView over SessionOverlay | Session Windows view and Running chip; `web` ChatPage over conversation.terminals |
| Transcript blocks, markdown and tool output | Shared agent renderers under AgentMessageList | Session page Blocks (single blocks), Turns and Stream (AgentMessageList over `useTurnFlow`), Session (ChatSession over `useTurnFlow`); Markdown and Code |
| Fold: height and chevron | ui/Fold, ui/FoldChevron | Motion Fold specimen; Skills packs; FunctionalBlock |
| Settings shell, rail, pages, groups, rows, split lists | settings/SettingsDialog, sections (`SETTINGS_SECTIONS`), SettingsPage, SettingsGroup, SettingsRow, SettingsSplit, SettingsListItem | Settings full mock and narrow variant; the product settings in `web` |
| Settings pages: General (theme, tone, accent, text size), Account, Archived, Keyboard; Notifications and Data & privacy pages exist as specimens behind disabled rail entries | settings/SettingsGeneral, SettingsAccount, SettingsNotifications, SettingsArchived, SettingsKeyboard, SettingsData | Settings full mock over fixture state, where text size applies `applyTranscriptTextSize` and theme, tone and accent drive the gallery's own appearance axes; `web` over backend preferences, where General applies appearance and Keyboard drives the shared application shortcut binding. A whole unused page is disabled on the rail; leftover rows on a usable page stay disabled with the In development tooltip |
| Stacked dialogs: a dialog opened inside another keeps it, Escape closes the top | ui/Dialog over overlayStore's `stacked` layer | Every settings page that opens a dialog, in the product |
| Shortcut notation: one string the recorder writes and the app matches | ui/shortcut (`formatShortcut`, `matchesShortcut`), ShortcutRecorder | Keyboard page; `web` `App` listens through the bindings |
| Devices: claim by pairing code, online status, revoke; cloud reset pending, refused, then phased | settings/SettingsDevices `resetPending` / `resetError` | Settings full mock (every second reset is refused) and the product Devices panel over `useDeviceSettings` |
| Models & providers: rail, provider page, add/model dialogs, login dialog | settings/SettingsProvidersPage, AddProviderDialog, ModelDialog, ProviderLoginDialog | GallerySettingsProviders over fixture state and mock handlers; `web` ProvidersPanel over the providers the composer also reads |
| MCP servers: status + tool tags, add dialog | settings/SettingsMcp, AddMcpServerDialog | Settings fixture; product entry disabled |
| Skills: git sources, folded packs, per-skill switches, pack enable switch | settings/SettingsSkills, AddSkillSourceDialog | Settings fixture; product entry disabled |
| Account credentials: change email (address, password, code), change password | settings/ChangeEmailDialog, ChangePasswordDialog, opened from SettingsAccount | GallerySettingsFull mock flows; standalone wells with every phase; the product Account page |
| Text entry: prefix/suffix, secrets with an eye, token counts with a unit, 36px lg for page forms | TextInput, TokenInput | Primitives specimens; Sign in uses TextInput/Button lg |
| Branching choice: cards with an icon, title and line | ui/ChoiceCards | Primitives specimen; the new-project form's Cloud or Device |
| File browser: folder and file choosing over a `FileBrowserSource`, address bar, places and devices, states | files/FileBrowser, FileBrowserDialog, FileIcon (see `docs/file-browser.md`) | Files page over fixture trees; the workspace picker, new-project Browse and the composer's remote attachment in `web` |
| Composer Add menu: local files, and a remote file when the conversation has a host | SessionComposer `remoteFiles` / `attachRemote` | GalleryComposer opens the Files dialog; both hosts add a reference tile |
| Composer attachment upload: determinate donut from 0 to 1, then ready; a failed upload leaves the composer with a toast | AttachmentTile, `ComposerAttachment` phase / progress / destination, SessionComposer send block | Session Composer specimens pin ready and a mid-upload ring; gallery uses timed fixtures; `web` uses actual byte-transfer progress |
| Session and list load: first opening uses the loading pane through history and initial connection; cached navigation reuses state and connection without reloading; an open session's dropped connection shows Connecting in the activity slot. Failed restore never reads as an empty conversation or first-run list. Retry reloads: loading, then ready | SessionStatus, `ConversationCache`, ActivitySlot, `SessionLoad` / `ListLoad`, AgentMessageList, AppSidebar `listStatus` | Session States view (cached switching uses ConversationCache and AgentMessageList; failed and missing are live; reconnecting pins ActivitySlot); Sidebar States loading / failed · live / first run, timed by the gallery's `RestoreSweep` fixture; `web` ChatPage and AppSidebar |
| Chrome row entrance: a thinking, tool or abort row that joins a live transcript, and the activity slot when it appears, slide in from the left while fading in; history and scroll remounts stay still | `useChromeEntrance` through AgentMessageList, `chrome-enter` on AgentMessageVirtualBlock and ActivitySlot | Motion page Entrance specimen; Session Turns and Session views live; `web` ChatPage |
| Activity slot: Connecting, Resuming, Retrying, Requesting faces; a thinking or tool block rolls into the row before its transcript row takes over | ActivitySlot, `activitySlotKind`, `useActivityHandoff`, `ConversationRuntime.resume()` setting `pendingAction`, all through AgentMessageList | Session Blocks view pins every face and an incoming thinking and shell block; Turns view plays turn, resume, retry and connect over `useTurnFlow` state; `web` ChatPage over the live runtime |
| Composer when no model can send: snackbar in the composer slot, Configure models. Last choice unusable: warning on the chip, send blocked, switch required. Model catalog failed to load: told beside the chip with Retry | SessionNoticeBar via SessionComposer, ModelSelector, `composerModel`, SessionComposer `modelLoad` / `retryModels` | Session Composer `model unavailable`, `no models` and `models failed · live`; Errors page; `web` ConversationComposer opens Models & providers and revalidates the catalog |
| Archived conversation: the same snackbar, Restore conversation | SessionNoticeBar via SessionComposer `archived` | Session Composer `archived` and `archived · live`; `web` ChatPage |

Toasts are only for outcomes with no other surface (a failed send, a rejected drop, a refused archive or environment change, copy after a menu closed). Success is silent — the list or field already shows it. Batch actions do not open a results sheet or offer retry-failed. Gallery mocks do not toast in-place successes or stand in for dialogs the product does not have; a taken shortcut uses `SettingsNote` on the Keyboard page.

## Product-flow coverage boundary

The gallery is a component catalog, not a second application with copied stores.
Host binding persistence, recent-directory selection, routing and the rich
seeded conversation scenarios are assembled
in `web`; the file browser those flows open is the shared component over a
`FileBrowserSource` the product supplies. The archive snackbar itself is shared;
`web` still owns when a conversation is archived. Their complete workflows must be
accepted in `web`; the gallery does not currently reproduce these composed
workflows. A component specimen alone does not establish end-to-end parity.

The host menu composition in Overlays demonstrates the current row structure;
its search picker is the actual shared component. The specimen does not connect
devices or mutate application bindings. When a product composition becomes a
reusable UI contract, extract it to `web-ui` and supply separate fixture/store
adapters in each consumer instead of copying the implementation. This is the
rule for every behavior, not only compositions: a control's affordance or a
page's interaction built for one surface is generalized into `web-ui` before
the checkpoint. The gallery's providers page is the model: `web-ui` owns the
page and its dialogs over `SettingsProviderEntry` models, and the gallery only
holds fixtures and mock handlers.

## Cloud lifecycle and reset acceptance

The shared Cloud settings presentation and reset dialog belong to `web-ui`.
The product supplies user Cloud state and backend handlers; gallery fixtures
supply the same contract. Cover sleeping, starting, ready, resetting and failed
states, including a guest that cannot connect. The reset dialog names its
user-wide impact, stops all Cloud tasks and preserves `/home`. Simulated success
must retain device/project identity and home files while replacing system state.
A metadata-only project deletion must leave the shared machine and files intact.
See `managed-hosts.md` and `product.md` for authoritative behavior.

The Session States page also embeds `GalleryConnectedSession`: the same
`ChatSession`, `SidebarLayout` and `WorkspaceDirectoryMenu` with `HostMenu`
nested in it, as the product's `WorkspaceInfo` composes them, over fixture
hosts, recent directories and handlers. Both composers use `RemoteFilePicker`;
media links and video use `ContentMedia` with shared cleanup.

The gallery never renders a transcript through a component of its own. Every
session specimen mounts `AgentMessageList` (directly or through `ChatSession`)
over state shaped like the product's conversation state; `turn-flow.ts` only
changes that state on a schedule, the way the runtime does on events. The tail
row, its faces, the handoff of a block into the row and the follow-on scrolling
are `web-ui`'s, so a motion seen in the gallery is the motion the product has.

The Session States “Scroll control without task chips” example uses `ChatSession`
and `SessionDock` with no task chips. Scrolling away from and back to the bottom
fades the left-aligned arrow inside a permanent 28px slot. The control row and its
8px gap remain part of dock height and transcript bottom padding, preserving the
composer position and scroll range. Product and gallery use the same layout.
