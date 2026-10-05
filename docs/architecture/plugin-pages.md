# Plugin pages

The web app is a shell, and plugins fill it. The shell holds what every
conversation needs: the application frame, the sidebar, the transcript and the
composer, the settings dialog with its core sections, and the frame of the
work panel. Every other surface is a plugin's **page**: the conversation
browser's tabs, the expose menu, the Skills settings and their sidebar entry,
the Change view and the File view. A page is written only against the
**plugin SDK**, `@demicodes/plugin-sdk`, so the shell knows no plugin and a
plugin knows no part of the shell beyond the SDK.

This document owns the page side of a plugin: how a page is declared, what it
can fill, what the shell gives it, how its data reaches it, and how it is
registered and versioned. [Plugins](plugins.md) owns the backend side: the
manifest, the contract and the plugin host.

## A link, end to end

For example, the agent's reply names `src/app.ts` and the user clicks it:

```text
transcript (shell)    the link names a Host file: open the `file` intent
                      with { path: "/home/ada/work/src/app.ts" }
work panel (shell)    the first enabled kind that opens `file` is the `file`
                      kind of the `file-browser` page, and it is pinned: the
                      panel asks the kind for its tab's next data, selects
                      that tab and opens the panel
`file` kind (plugin)  its content reads the file through the conversation's
                      files service and shows it, with Back to what it
                      showed before
```

With `file-browser` turned off, no enabled kind opens `file`, so the
transcript shows the path as text and offers nothing to click. Neither the
transcript nor the panel names the `file-browser` plugin.

## One declaration, in code

A plugin is declared once, as data in code: its crate's manifest
([What a plugin contributes](plugins.md#what-a-plugin-contributes)), a plain
value that names, among the rest, the plugin's page package and the schemas of
its page's state, calls and streams. The page package declares its
contributions to the shell as a plain object, its `PluginPage`, and nothing
about the plugin itself:

```text
crate plugin-browser            manifest: id "browser", packages, commands,
                                streams with their messages, page package
                                "@demicodes/plugin-browser", state schemas,
                                methods
        │ xtask contracts
        ▼
@demicodes/plugin-browser       src/generated/: the plugin's id, the types of
                                its state, calls and streams, their constants
        │ imports them
        ▼
src/index.ts                    export default definePage({ plugin, kinds, panel })
```

Everything else about a page is derived from the manifest: its plugin's id,
which the page imports from its generated module rather than spelling it, its
types, and its place in the [registry](#registration). A plugin that only
shows a page, such as `changes`, still has a crate, whose manifest declares
its identity and its page package and nothing else.

The declaration is code, not a separate file such as a `plugin.json`, because
a manifest refers to what only code holds: its schemas are derived from the
Rust types the plugin uses, and a page's contributions are its Vue
components. A file would restate both as text, and nothing would check that
the text still matched them.

## Packages

```text
web (the product)                         web-gallery
  services over the backend                 fixture services
         \                                    /
          +--> generated registry <----------+
                     |
          plugin-browser  plugin-expose  plugin-skills  plugin-changes  plugin-file-browser
                     |    a plugin's feature UI: its components and its state
                     v
               plugin-sdk     definePage, usePage, intents, services, the plugin kit
                     |
                     v
                  web-ui      the shell's components and the primitives the kit exposes
```

- **`plugin-sdk`** is the whole API a page may use. A page package imports
  `@demicodes/plugin-sdk`, `@demicodes/utils`, Vue, Zod and its own generated
  module, and nothing else of the workspace; the
  [package boundary check](crates-and-packages.md#boundary-checks) enforces it.
- **A page package**, `@demicodes/plugin-<name>` in `packages/plugin-<name>`,
  holds the plugin's feature UI: its components, their state and their
  handlers.
- **`web-ui`** holds the shell's components and the reusable primitives. It
  knows no plugin.
- **`web` and `web-gallery`** supply the services, over the backend or over
  fixtures, and show the pages the registry lists.

## The page object

A page is one object, made with `definePage`, that names its plugin and says
what it contributes:

| Field | Contributes | Example |
| --- | --- | --- |
| `plugin` | Its plugin's id, imported from the generated module | — |
| `settings` | A section of the settings dialog: its entry in a group of the rail, and its page | `skills` |
| `settings.sidebar` | An entry in the sidebar's top group that opens the settings dialog on the section | `skills` |
| `headerTool` | A component the conversation header shows; it shows nothing while it has nothing to show | `expose` |
| `kinds` | The [kinds](#work-panel-kinds) of tab it shows in the work panel | `browser`, `page`, `change`, `file` |
| `panel` | What runs for a conversation while its panel is open with the plugin on, such as binding the tabs the agent opened ([Panel sessions](#panel-sessions)) | `browser` |

Every field but `plugin` is optional, and all of them are fixed: a page
declares what it fills, not what it does at a given moment. A field fills its
slot only while the user has the plugin on, in the order the backend registers
its plugins. These are the slots there are; a slot joins the SDK when a
feature needs it, not before.

A sidebar entry is a way into a settings section, not a surface of its own,
so a page declares it on its section, as `sidebar: true`, and the entry shows
the section's icon and label. A page without a section has no entry, and the
sidebar and the rail never name one section two ways. The top group shows the
shell's New, then the entries in registration order, then the shell's
Archived; clicking an entry opens the settings dialog on its section, as
Archived opens its own. For example, with `skills` on the group reads New,
Skills, Archived, and with it off, New, Archived.

A slot's component receives as props only what that slot is about: a header
tool its `conversation`; a kind's content its `conversation`, its page's
panel `session` of that conversation, its `tabId`, `data` and `shown`; a
settings section nothing. Everything else it reaches through
[`usePage()`](#the-page-context).

## The page context

Every component of a page, in any slot, reaches its plugin and the shell
through one call, `usePage()`, which the shell binds to the page's own plugin.
A component never names a plugin, and cannot reach another plugin's state or
calls.

| Part | Gives | Supplied by `web` over |
| --- | --- | --- |
| `plugin` | The plugin's user state and its user calls; `plugin.conversation(id)` its conversation state, its conversation calls and its user streams ([Data a page shows](#data-a-page-shows)) | The sync channel, the conversation state route, the plugin call routes, the user stream route |
| `intents` | Opening an [intent](#intents) for a conversation, as `{ intent, payload }`, and whether any page the user has on opens it | The shell |
| `panel` | The tabs of the page's own kinds in a conversation's panel, and adding one, selected with the panel opened or not | The shell |
| `settings` | Opening a section of the settings dialog, such as Devices | The shell |
| `errors` | Reporting an error the user sees, or a defect of the page, which only the console shows | The shell |
| `overlays` | The overlay store a dialog or menu opens in | The shell |
| `files(conversation)` | The conversation's [files service](#the-conversation-files-service) | The file routes |

The gallery supplies the same context over each specimen's fixture state, so
every control of a specimen acts on that state.

## Work panel kinds

A kind is declared once, on the page, as data: what the panel needs to show
its tabs.

| Field | Meaning |
| --- | --- |
| `kind` | Its id, unique across plugins; the panel refuses a duplicate |
| `schema` | The schema of a tab's `data`, checked where the backend's panel enters the page |
| `title(data, tab)`, `mark` | The tab's title, from its data and its conversation and panel session, and its strip mark (props: `data`) |
| `content` | The tab's content (props: `conversation`, `session`, `tabId`, `data`, `shown`; emits `update` with the next `data`, and `close`) |
| `create` | Whether the strip's new-tab control offers it, with its label, icon and a new tab's `data` |
| `pinned` | The kind has one tab in every conversation's panel, ahead of the other tabs, never created, closed or kept by the backend; its data starts from the data this gives and lives in the page's memory; its id is the kind's id |
| `picked(data)` | What a tab shows next when its user picks it in the strip, even while it is selected: the Change view returns to Uncommitted ([Delivery to the conversation](../execution/edit-tracking.md#delivery-to-the-conversation)) |
| `badge` | What the strip shows after a pinned tab's title, such as the Change view's counts (props: `conversation`, `data`) |
| `intents` | The [intents](#intents) it opens: for each, the data its tab shows next, from the payload and the data the tab shows now, or none |

Because a kind is data on the page, the panel knows every kind of every page
the user has on before it opens, and an intent can open a kind while the panel
is closed. A kind that is not pinned is also named in its plugin's manifest,
since the backend keeps its tabs, and what follows from a tab being created
or closed, such as the browser closing its own tab, is the plugin's work on
the backend ([Panel kinds](plugins.md#panel-kinds)), never the page's.

### Panel sessions

Some kinds need something running for a conversation while its panel is open:
the `browser` page keeps the conversation's one live view. A page's `panel` makes that for a
conversation, with the page's context, when the conversation's panel opens
with the plugin on, and ends it when the panel closes, shows another
conversation or the plugin turns off. It runs in an effect scope of its own,
so what it follows stops with it, and its `dispose`, if it has one, is called
then. The page's kinds of that conversation receive it: their functions as
`tab.session`, their contents as the `session` prop. A session reads the tabs
of the page's own kinds and adds tabs of them; it holds nothing the backend
keeps.

## Intents

An intent is a request to show something, named by what it shows rather than
by who shows it. The SDK defines each intent and its payload:

| Intent | Payload | Opened by the shell from |
| --- | --- | --- |
| `file` | `{ path }`, an absolute path on the conversation's Host | A file a message names ([Files named in messages](../product/file-previews.md#files-named-in-messages)), an attachment's capsule, a Host image |
| `edit` | One call's edit of one file, as the transcript's tool block names it | A tool call's file pill, a message edit's selection |

To open an intent, the panel takes the first kind, in registration order, of
a page the user has on that declares it. A pinned kind's tab takes the data
the kind returns and is selected; another kind gets a new tab with that data,
selected. The panel opens if it was closed. Whoever shows a control that opens
an intent asks first whether any page opens it, and shows plain text
otherwise.

Intents and `panel.add` are the only ways a tab is opened from outside the
strip, and `panel.add` adds only a tab of the page's own kinds: a page opens
another plugin's tab only through an intent.

## Data a page shows

Everything a page shows is pushed to it, or read when a pushed revision says
it changed; no page polls, and no page guesses when to read again.

- **User state.** A plugin's state for its user reaches every page on the
  sync channel, as part of the product state, whenever the plugin marks it
  changed ([Page synchronization](../product/web-api.md#page-synchronization)).
  The Skills settings and the expose menu show it.
- **Conversation state.** A plugin's state for one conversation, such as the
  conversation browser's tab list, can be large, and only the pages that show
  that conversation need it. So it travels as a conversation's draft does:
  the conversation's summary carries the revision of each plugin's
  conversation state, and a page that shows the conversation reads that state
  when the revision is higher than the one it holds
  ([Conversation state of plugins](../product/web-api.md#conversation-state-of-plugins)).
- **When a plugin's state changes.** A plugin marks a scope of its state
  changed through its port, after a call of its own changed it, or when a
  topic it follows fires: the user's exposes for the expose menu, a
  conversation's jobs ending for the browser's tabs, which the agent's
  commands open and close ([Topics](plugins.md#topics)).
- **Product services keep themselves fresh.** While a component that shows
  the working tree says so with `showChanges()`, the conversation's files
  service lists it again when the conversation's working-tree revision
  changes, after any job ended, and when the page is shown again, since the
  user may have changed files outside Demi meanwhile. A page that shows the
  files reads the service, and never decides when to read again.

## Calls and states

A page reads its plugin's states and answers with the schemas generated from
the plugin's manifest ([Types](#types)), and the page context checks each one
where it enters the page. For example, the Skills page's schema says that
every source has `updateAvailable`. A backend of an older build sends the
skills state without it: the page context does not hand that state to the
Skills page, which keeps showing the last state that read, and a toast says
in plain words that the page cannot read the plugin's data and that reloading
the page may help. What did not read, here the missing field, is for a
developer: the page writes it to the browser console, never into the toast.

- **A state that does not read is reported, never thrown.** Reading a state
  never throws into a component. A component that throws while it renders
  stops updating: in development Vue keeps the DOM of its last render, so a
  switch that showed its call pending stays pending; in a production build
  the component shows nothing. So the page context keeps the last state that
  read, or none before the first, and says why the latest did not read until
  one does. A conversation state has a place on its page, so its error is
  the state's `error`, which the page shows where the state shows, with its
  retry. The user state has no such place, so a toast says it once, when the
  page stops reading it.
- **An answer that does not read fails its call**, as a refusal does, with
  what did not read as its message.
- **A control waits for its own call, and only for it.** A control that
  starts a call, such as a skill's switch, is pending from the user's action
  until that call is answered, whether it succeeds or fails, and takes no
  input meanwhile; it never waits for the state the call causes. On success
  the control shows the state the plugin sends; on failure the page reports
  the call's error, as `errors.report` reports any failed call, in a toast,
  and the control shows the plugin's last state again. When the component
  that showed the control goes, the calls it still waits for are aborted and
  nothing reports them. The SDK's `pendingCalls` is that rule, by the key of
  what each control acts on: the Skills page's switches by their source and
  the expose menu's rows by their expose.

## The conversation files service

A conversation's files are a product service, not a plugin's: the runner
records every job's edits and the backend serves every read of a Host file,
for whichever page shows them. `usePage().files(conversation)` gives:

| Part | Gives |
| --- | --- |
| `workspace` | The Host's file tree and file contents, with the workspace's root and name, while the conversation's Host is known |
| `root` | Where the conversation's work runs, which a retained edit's paths are relative to |
| `changes` | The working tree's uncommitted changes: the list, each file's two sides and the committed contents, with whether a listing is on its way or failed |
| `edit(copies)` | The two sides of one call's retained edit |
| `showChanges()` | The calling component shows the working tree: the service keeps `changes` fresh until the component's scope ends ([Data a page shows](#data-a-page-shows)) |

`changes` and `file-browser` show these; any other page may read them.

## The plugin kit

The SDK's components are a kit, chosen as a set rather than by what one page
happened to use, and the SDK's entry exports them by these groups:

| Group | Components |
| --- | --- |
| Settings | `SettingsPage`, `SettingsGroup`, `SettingsRow` |
| Controls | `Button`, `IconButton`, `Switch`, `TextInput`, `Dropdown`, `Menu`, `MenuItem`, `MenuGroup`, `MenuDivider`, `Popover`, `Tooltip`, `Dialog`, `Fold`, `FoldChevron`, `ExternalLink` |
| Progress | `IndeterminateSpinner` |
| Navigation | `AddressBar` |
| Files | `FileIcon`, `FileView`, `ChangeView`, and the shapes and paths the conversation files service gives |
| Icons | `ICON_PX`, the icon sizes, and Demi's own icons, such as `GlobePlus` |
| Composables | `useTimeRemaining` and its formatter; `pendingCalls`, the calls a page's controls wait for ([Calls and states](#calls-and-states)) |
| Streams | The liveness helpers a stream's protocol uses to tell a silent stream, and the waits before opening one again |

A primitive joins the kit when a page needs it and another page could use it;
one only a single page could use stays in that page.

## Types

A page's types are generated from its manifest into its package's
`src/generated/`: its plugin's id; the schemas of its user and conversation
state, of its methods' parameters and results, and of its streams' messages,
such as the live view's frames; and the constants a stream's two ends share.
`xtask contracts` reads them from the manifest and keeps no list of its own. A
page never declares a schema by hand for a shape its manifest declares
([Generated TypeScript](contracts.md#generated-typescript)).

## Registration

`xtask contracts` writes the registry, the static list of imports that `web`
and `web-gallery` show: the page package of each plugin the backend registers
whose manifest names one that the app depends on, in the order the backend
registers their plugins, which is the order of the slots they fill. Adding a
plugin's page to the product is adding the dependency; no import is written by
hand, and no import is computed.

## Versions

The SDK's major version is the page API's version. A change that could break
a page written against it, such as a slot, a part of the page context, an
intent or a kit component removed or changed incompatibly, raises the major
version; adding one does not. The pages of this repository always build
against the SDK beside them. A page built elsewhere is checked against the
major version it declares, with the loading below.

## Pages from outside the repository

Open, decided with the TypeScript SDK ([Decisions for the TypeScript SDK](plugins.md#decisions-for-the-typescript-sdk)).
What the design above already keeps possible:

- A page from outside is an ES module built against one SDK major version,
  with Vue and the SDK supplied by the page around it; the backend serves it
  with the plugin.
- Every part of the page context passes data, so a page whose code nobody
  reviewed can run in a sandboxed frame, with `usePage()` over messages to the
  shell, without changing the page's code.
