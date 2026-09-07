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
| Settings shell, pages, groups, rows, split lists | settings/SettingsDialog, SettingsPage, SettingsGroup, SettingsRow, SettingsSplit, SettingsListItem | Settings full mock and narrow variant |
| Models & providers: rail, provider page, add/model dialogs, login dialog | settings/SettingsProvidersPage, AddProviderDialog, ModelDialog, ProviderLoginDialog | GallerySettingsProviders over fixture state and mock handlers |
| Account credentials: change email (address, password, code), change password | settings/ChangeEmailDialog, ChangePasswordDialog | GallerySettingsFull mock flows; standalone wells with every phase |
| Text entry: prefix/suffix, secrets with an eye, token counts with a unit | TextInput, TokenInput | Primitives specimens |
| File browser: folder and file choosing over a `FileBrowserSource`, address bar, places and devices, states | files/FileBrowser, FileBrowserDialog, FileIcon (see `docs/file-browser.md`) | Files page over fixture trees; the workspace picker, new-project Browse and the composer's remote attachment in `web` |
| Composer Add menu: local files, and a remote file when the conversation has a host | SessionComposer `remoteFiles` / `attachRemote` | GalleryComposer opens the Files dialog; `web` inserts the path into the draft |

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
