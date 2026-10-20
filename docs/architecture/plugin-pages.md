# Plugin pages

The web app is a shell, and plugins fill it. The shell holds what every
conversation needs: the application frame, the sidebar, the transcript and the
composer, the settings dialog with its core sections, and the frame of the
work panel. Every other surface is a plugin's **page**: the conversation
browser's tabs, the Skills settings and their sidebar entry,
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
          plugin-browser  plugin-skills  plugin-changes  plugin-file-browser
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
| `kinds` | The [kinds](#work-panel-kinds) of tab it shows in the work panel | `browser`, `change`, `file` |
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
| `plugin` | The plugin's user state and its user calls; `plugin.conversation(id)` its conversation state, its conversation calls, its user streams, and `hostStarting`, whether the conversation's Host is starting, as a waking Cloud is ([Data a page shows](#data-a-page-shows)) | The sync channel, the conversation state route, the plugin call routes, the user stream route |
| `intents` | Opening an [intent](#intents) for a conversation, as `{ intent, payload }`, and whether any page the user has on opens it | The shell |
| `panel` | The tabs of the page's own kinds in a conversation's panel, and adding one, selected with the panel opened or not | The shell |
| `settings` | Opening a section of the settings dialog, such as Devices | The shell |
| `links` | Opening a web page in a new tab, such as a vendor's sign-in page: `open(url)` takes the URL or a promise of it, so the shell opens the tab on the click and points it at the URL once the promise settles, before a popup blocker would refuse a later open; the gallery shows a toast naming the page instead | The shell |
| `errors` | Reporting an error the user sees, or a defect of the page, which only the console shows | The shell |
| `overlays` | The overlay store a dialog or menu opens in | The shell |
| `files(conversation)` | The conversation's [files service](#the-conversation-files-service) | The file routes |

A call the page cannot send because the backend cannot be reached, as while
the server restarts, waits until the page reaches the backend again and is
sent then, as every control does under the
[connection banner](../product/web-application.md#a-page-of-another-build):
a page never shows a failure about the connection, and a call fails only for
the backend's own answer. A read of a plugin's state waits the same way. The
backend's `503 backend_closing`, which it answers while it shuts down without
doing the request, and a proxy's 502, 503 or 504, or Cloudflare's 520 to 524,
which it answers when the origin gave it nothing usable, without the
backend's error body count as not reached. A call whose caller goes away, as a panel that
closes, is dropped and never sent. For example, the user presses Reload in a browser
tab while Demi restarts; the tab shows it loading, and the reload reaches
the browser once Demi is back.

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
| `content` | The tab's content (props: `conversation`, `session`, `tabId`, `data`, `shown`, whether it is the tab the panel shows: a content stays mounted while another tab is selected ([Work panel](../product/web-application.md#work-panel)); emits `update` with the next `data`, and `close`) |
| `pinned` | The kind has one tab in every conversation's panel, ahead of the other tabs, never created, closed or kept by the backend; its data starts from the data this gives and lives in the page's memory; its id is the kind's id |
| `icon(data)` | The strip's icon for a tab, such as a page's favicon; the kind's own icon otherwise |
| `commands(data)` | The kind's own entries in a tab's menu, ahead of the strip's Close entries, such as a browser tab's Reload |
| `badge` | What the strip shows after a pinned tab's title, such as the Change view's counts (props: `conversation`, `data`) |
| `key(data)` | What makes a tab of a kind that is not pinned the one it is, such as an attachment's number; an intent whose new data has the key of a tab the strip already holds selects that tab instead of adding another, as clicking a card a second time shows its file's tab again. A kind without it, such as the browser's, opens a new tab each time, as a browser does |
| `intents` | The [intents](#intents) it opens: for each, the data its tab shows next, from the payload and the data the tab shows now, or none |
| `shows(data)` | How many times something asked that the user see this tab, from its data. When the count is higher than the one the page last applied for the tab, the panel opens and selects the tab, whether the panel was open or closed, and records the count beside its selection history; the browser's agent-shown tabs use it (`crates/plugin-browser/src/panel.rs`) |

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
| `file` | `{ path, tree? }`, an absolute path on the conversation's Host, whose row the File view's tree selects and shows ([The File view's bars](../product/file-previews.md#the-file-views-bars)); `tree: true` also shows the tree if it was hidden | A file a message names ([Files named in messages](../product/file-previews.md#files-named-in-messages)), an attachment's capsule, a Host image; *Show in Files* on a file link |
| `web` | `{ url }`, an `http` or `https` address | *Open in Work Panel* on a link in a reply ([Context menus](../product/web-application.md#context-menus)) |
| `attachment` | `{ id, name, size, url }`: the attachment's number, its file name, its size in bytes and the address of its stored copy | An attachment's card ([Attachment blocks](../product/file-previews.md#attachment-blocks)) |
| `edit` | A request's changes: the agent (`node`, null for the conversation's own), the request, and a file with one call's edit of it (`{ call, segment }`) or null for All Changes ([A request](../execution/edit-tracking.md#a-request)) | A tool call's file pill, a request's changed-files line |

To open an intent, the panel takes the first kind, in registration order, of
a page the user has on that declares it. A pinned kind's tab takes the data
the kind returns and is selected; another kind selects the tab of the same
`key`, when the strip holds one, and otherwise gets a new tab with that data,
selected. The panel opens if it was closed. Whoever shows a control that opens
an intent asks first whether any page opens it, and shows plain text
otherwise.

Intents are the only way a page opens a tab: a page opens another plugin's
tab only through an intent, and a plugin's own tabs are opened by the
plugin. `panel.select` selects a tab of
the page's own kinds that the strip already holds, as a browser selects the
tab a click in a page opened.

## Data a page shows

Everything a page shows is pushed to it, or read when a pushed revision says
it changed; no page polls, and no page guesses when to read again.

- **User state.** A plugin's state for its user reaches every page on the
  sync channel, as part of the product state, whenever the plugin marks it
  changed ([Page synchronization](../product/web-api-sync.md#page-synchronization)).
  The Skills settings show it.
- **Conversation state.** A plugin's state for one conversation, such as the
  conversation browser's tab list, can be large, and only the pages that show
  that conversation need it. So it travels as a conversation's draft does:
  the conversation's summary carries the revision of each plugin's
  conversation state, and a page that shows the conversation reads that state
  when the revision is higher than the one it holds
  ([Conversation state of plugins](../product/web-api-sync.md#conversation-state-of-plugins)).
- **When a plugin's state changes.** A plugin marks a scope of its state
  changed through its port, after a call of its own changed it, or when its
  command package tells it the state on a Host changed: the browser's tabs,
  which the agent's commands and the pages themselves open and close
  ([Messages from its command package](plugins.md#messages-from-its-command-package)).
- **Product services keep themselves fresh.** While a component shows the
  conversation's files, the conversation's files service follows the Host's
  reports of what changed and reads again what they name
  ([What the service keeps](#what-the-service-keeps)). A page that shows the
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
  and the control shows the plugin's last state again. A call is an
  operation of the page ([Responding to the user](../product/web-application.md#responding-to-the-user)),
  by the plugin's action and the key of what it acts on, such as the Skills
  page's switch of a source: when the component that showed the control
  goes, the call goes on, and the control drawn again shows it working. The
  page context's `operation(action, key)` gives a call's operation, and
  `Button`, `IconButton`, `MenuItem` and `Switch` take it; the SDK has no
  pending state of its own. A plugin's switch waits for its call and then
  shows the state the plugin sends, since that state is the plugin's to
  tell, not the page's. A control that only sends input to what the user
  watches, such as a simulator's Home, is the exception: it waits for
  nothing, since the picture shows the effect, and a failure is reported
  in a toast ([The live view](../mobile/mobile.md#the-live-view)).

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
| `request(node, request)` | A request's changes, derived from that agent's transcript: its files in first-changed order, each with its edits ([A request](../execution/edit-tracking.md#a-request)) |
| `showChanges()` | The calling component shows the working tree: the service follows the Host's reports for it and keeps `changes` current until the component's scope ends ([What the service keeps](#what-the-service-keeps)) |

`changes` and `file-browser` show these; any other page may read them.

### What the service keeps

The service keeps what it read and learns from the Host's own file system
when that changes, the way VS Code, JetBrains IDEs and Zed keep a remote
workspace: it subscribes to what it keeps, and the Host says when a
subscription's subject changed ([Watching files](../execution/runner.md#watching-files)).
For example, the user reads `src/app.ts`, picks `README.md`, and picks
`src/app.ts` again: its text shows at once, with its scroll where it was, and
nothing is asked, since its subscription did not change. The agent then edits
`src/app.ts` and runs `cargo check`: the file's subscription and the working
tree's change, the shown text is read again and replaced in place, without a
loading state, and the 20,000 files the build writes under `target/` change
nothing and read nothing.

- **What it keeps.** Directory listings, file texts, file descriptions, the
  changes list, each changed file's two sides and its committed contents,
  each as the last answer read, keyed by the Host and the absolute path.
  Conversations on one Host share them. They live in the page's memory for
  the signed-in page's lifetime and are gone after a reload, as the
  conversation cache is
  ([Backend communication](../product/web-application.md#backend-communication)).
  At most 64 MiB and 4,096 entries of them stay, counted by their text; past
  that the ones shown longest ago go first. An entry holds what was read,
  never a way to read it: each view reads through its own route, a
  conversation's views through the conversation's host access or its direct
  channel, a device's folder dialog through device access, and every view
  that shows the entry sees what any of them read. A read nobody asked for,
  after a change or Refresh, goes through the view that came on screen last.
- **Every answer is a result.** A text, a file that is binary, one too large
  to read as text, and one that is gone are each an answer with its version
  and its number, kept and confirmed alike, and a view draws each as itself:
  a binary file's card, a too-large file's notice. Only a read that could not
  be made, a Host out of reach or a refusal, is a failure.
- **A view keeps its place.** No update remounts or resets a view. A view is
  keyed by what the user chose, such as the mode, the file, the edit or All
  Changes, never by the version of its content. An update that changes
  nothing the view shows changes nothing on screen; one that changes its
  content replaces it in place, both sides of a diff in one step, with the
  line at the top of the window kept where it is, as an editor keeps its
  place when the file changes on disk. Reading again never clears what is
  shown: a binary file's card stays while it is checked. Only another choice
  of the user's, or Reload, shows a loading state; a loading state never
  replaces content already shown. A value the page derives anew on each
  frame, such as the transcript, is compared by what it holds, never by its
  identity, so a new frame that changes nothing is no change.
- **Everything kept is subscribed; only what is shown is read.** Each entry
  holds the subscriptions it depends on for as long as it is kept, whether a
  view shows it or not, and gives them up when it leaves the cache:

  | Entry | Subscriptions |
  | --- | --- |
  | A file's text or description | `file` |
  | A folder's listing in the file tree | `folder_names`: a file written in place changes no name, so its listing stays |
  | A folder's listing with sizes and times, in the attachment picker or a device's folder dialog | `folder_details`; a listing both hold takes this one |
  | The changes list of a working tree | `worktree` |
  | A changed file's two sides, the last commit's and the working tree's | `file` and `head` |
  | A file's committed contents | `head` |

  A change of an entry on screen reads it again; a change of an entry off
  screen, in a work panel tab the user switched away from or a panel that is
  closed, is only noted, and the entry is read when it comes on screen. The
  Host keeps nothing per page and pushes no content: a change says only that
  the answer kept may be out of date, and the answer read is always the
  Host's whole truth.
- **Whether an entry is current is worked out from numbers, never stored.**
  Each answer carries the Host's number taken before it read, and each
  subscription its `since` and the `at` of its last change
  ([Numbers](../execution/runner.md#watching-files)). An entry is current when
  its answer has the Host's present epoch, and each of its subscriptions is
  live, started at or before the answer's number, changed last at or before
  it, and within its lease as the page counts it
  ([File watch](../product/web-api-hosts.md#file-watch)). For example, an answer read
  at 40 and a change at 41 make the entry out of date, whichever reached the
  page first; an answer read at 41 already holds a change at 41, and nothing
  is read again. Showing a current entry asks nothing.
- **Reading again costs little, and an unchanged answer changes nothing.**
  A file text, a listing, the changes list and a changed file's sides are
  read again with the version the service holds (`If-None-Match`), and one
  that has not changed answers that it has not, without its content; the
  view then keeps what it shows untouched. The changes list is read again at
  most once a second, and a folder's listing too, as an editor's source
  control view and Finder refresh while a build writes files: each changes
  list is a status over the whole repository, which a large one cannot answer
  many times a second. A change within that second is read with the ones after
  it when the second ends, so the last is never dropped. Nothing is read on a
  timer of its own: every read answers a change, a view that comes on screen,
  or the user's Refresh.
- **Out-of-date entries are shown, then checked.** An entry that is not
  current, after a change, a lapsed lease, a new epoch or a watch stream that
  reconnected, shows at once and is read again in the background; an answer
  that differs replaces it in place. Reads of one thing share one request,
  whichever view's route it took.
- **Leases.** The page renews every subscription its cache still holds in one
  message every 3 minutes; the lease runs 10 minutes, which outlasts the
  minute a background tab's timers may be held. A subscription whose lease
  the page counts as over makes its entry out of date, and the page subscribes
  again while the entry is kept. One that is refused, past the page's 4,096
  subscriptions or 64 outside the working tree, leaves its entry out of date,
  checked whenever it is shown.
- **A Host that cannot watch** a subject, past an inotify limit or on a file
  system that reports nothing, says so for that subscription alone. A view
  whose entry has no live subscription says quietly that it shows the file as
  it was last read and offers Refresh, and checks it each time it is shown;
  every other entry stays as it is.
- **Replacing in place.** The view keeps its scroll, selection and folded
  rows; a text view shows the new text as an editor reloads a file it has not
  changed. An answer that the file or folder is gone shows that, as a first
  read would. A read that fails keeps what it shows and says quietly that it
  could not refresh, with Retry.
- **The page's own writes** edit the listing they change at once, without
  listing it: an upload adds the entry with the name and size it sent, a new
  folder adds itself, a deletion removes its entry. The subscription's change
  then reads the folder once.

## The plugin kit

The SDK's components are a kit, chosen as a set rather than by what one page
happened to use, and the SDK's entry exports them by these groups:

| Group | Components |
| --- | --- |
| Settings | `SettingsPage`, `SettingsGroup`, `SettingsRow` |
| Controls | `Button`, `IconButton`, `Switch`, `TextInput`, `Dropdown`, `Menu`, `MenuItem`, `MenuGroup`, `MenuDivider`, `Popover`, `Tooltip`, `Dialog`, `Fold`, `FoldChevron`, `ExternalLink` |
| Context menus | `vContextMenu`, which declares an element's entries for the page's context menu ([Context menus](../product/web-application.md#context-menus)); a view that handles its own pointer, such as a remote page, stops the event itself |
| Progress | `IndeterminateSpinner`, `ProgressLine` |
| Status | `RegionStatus`, what a region shows in place of content it cannot show (the gallery's Errors page) |
| Layout | `ScrollArea` |
| Navigation | `AddressBar` |
| Files | `FileIcon`, `FileView`, `ChangeView`, and the shapes and paths the conversation files service gives |
| Icons | `ICON_PX`, the icon sizes |
| Streams | The liveness helpers a stream's protocol uses to tell a silent stream, and the waits before opening one again |

A primitive joins the kit when a page needs it and another page could use it;
one only a single page could use stays in that page.

## Types

A page's types are generated from its manifest into its package's
`src/generated/`: its plugin's id; the schemas of its user and conversation
state, of its methods' parameters and results, and of its streams' messages,
such as the live view's frames; the closed set of reasons its calls and
streams can fail with, which the page words in full
([Plugin calls](../product/web-api-sync.md#plugin-calls)); and the constants a
stream's two ends share.
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
