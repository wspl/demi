# Web and gallery synchronization

`web` and `web-gallery` consume reusable visual behavior from `web-ui`.
Neither product imports the other. A shared UI change includes both its product
usage and a gallery specimen in the same checkpoint; verification runs all three
browser package typechecks and checks affected specimens in the browser.

## Coverage

| Surface | Shared implementation | Gallery coverage |
| --- | --- | --- |
| Sidebar header, project names, pin/archive actions | AppSidebar, SidebarProjectHeader, SidebarRow | Sidebar live specimen |
| Marquee, read indicators, ordering, temporary folding and centered scrolling | Sidebar components and drag controller | Sidebar live specimen with mutable data |
| Menu sizing, label/value rows, section headings and status indicators | Menu, MenuItem, MenuGroup | Overlays specimens |
| Host search, Cloud option and bound-device disabling | hosts/HostPicker | Main and attached picker specimens in Overlays |
| Composer, model menu, attachments and input layout | SessionComposer, ModelMenu | GalleryComposer in Session |
| Transcript blocks, markdown and tool output | Shared agent renderers | Session, Markdown and Code |
| Fold: height and chevron | ui/Fold, ui/FoldChevron | Motion Fold specimen; Skills packs; FunctionalBlock |
| Settings shell, rail, pages, groups, rows, split lists | settings/SettingsDialog, sections (`SETTINGS_SECTIONS`), SettingsPage, SettingsGroup, SettingsRow, SettingsSplit, SettingsListItem | Settings full mock and narrow variant; the product settings in `web` |
| Settings pages: General (theme, tone, accent, text size), Account, Notifications, Keyboard, Data & privacy | settings/SettingsGeneral, SettingsAccount, SettingsNotifications, SettingsKeyboard, SettingsData | Settings full mock over fixture state; `web` over `prototype/settings.ts`, where General writes the document and Keyboard drives `App`'s shortcuts |
| Stacked dialogs: a dialog opened inside another keeps it, Escape closes the top | ui/Dialog over overlayStore's `stacked` layer | Every settings page that opens a dialog, in the product |
| Shortcut notation: one string the recorder writes and the app matches | ui/shortcut (`formatShortcut`, `matchesShortcut`), ShortcutRecorder | Keyboard page; `web` `App` listens through the bindings |
| Devices: claim by pairing code, online status, revoke | settings/SettingsDevices | Settings full mock and the product Devices panel |
| Models & providers: rail, provider page, add/model dialogs, login dialog | settings/SettingsProvidersPage, AddProviderDialog, ModelDialog, ProviderLoginDialog | GallerySettingsProviders over fixture state and mock handlers; `web` ProvidersPanel over the providers the composer also reads |
| MCP servers: status + tool tags, add dialog | settings/SettingsMcp, AddMcpServerDialog | Settings full mock; the product MCP page |
| Skills: git sources, folded packs, per-skill switches, pack enable switch | settings/SettingsSkills, AddSkillSourceDialog | Settings full mock; the product Skills page |
| Account credentials: change email (address, password, code), change password | settings/ChangeEmailDialog, ChangePasswordDialog, opened from SettingsAccount | GallerySettingsFull mock flows; standalone wells with every phase; the product Account page |
| Text entry: prefix/suffix, secrets with an eye, token counts with a unit | TextInput, TokenInput | Primitives specimens |
| File browser: folder and file choosing over a `FileBrowserSource`, address bar, places and devices, states | files/FileBrowser, FileBrowserDialog, FileIcon (see `docs/file-browser.md`) | Files page over fixture trees; the workspace picker, new-project Browse and the composer's remote attachment in `web` |
| Composer Add menu: local files, and a remote file when the conversation has a host | SessionComposer `remoteFiles` / `attachRemote` | GalleryComposer opens the Files dialog; `web` inserts the path into the draft |

Toasts are only for outcomes with no other surface (a failed send, a rejected drop, copy after a menu closed). Gallery mocks do not toast in-place successes or stand in for dialogs the product does not have; a taken shortcut uses `SettingsNote` on the Keyboard page.

## Product-flow coverage boundary

The gallery is a component catalog, not a second application with copied stores.
Host binding persistence, recent-directory selection, archive/composer
switching, routing and the rich seeded conversation scenarios are assembled
in `web`; the file browser those flows open is the shared component over a
`FileBrowserSource` the product supplies. Their complete workflows must be
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
