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

## Product-flow coverage boundary

The gallery is a component catalog, not a second application with copied stores.
Host binding persistence, recent-directory selection and its file browser,
archive/composer switching, routing, provider settings and the rich seeded
conversation scenarios are assembled in `web`. Their complete workflows must be
accepted in `web`; the gallery does not currently reproduce these composed
workflows. A component specimen alone does not establish end-to-end parity.

The host menu composition in Overlays demonstrates the current row structure;
its search picker is the actual shared component. The specimen does not connect
devices or mutate application bindings. When a product composition becomes a
reusable UI contract, extract it to `web-ui` and supply separate fixture/store
adapters in each consumer instead of copying the implementation.
