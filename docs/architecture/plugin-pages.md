# Plugin pages

The web app is a shell, and plugins fill it. The shell holds what every
conversation needs: the application frame, the sidebar, the transcript and the
composer, the settings dialog with its core sections, and the frame of the
work panel. Every other surface is a plugin's **page**: the conversation
browser's tabs, the expose menu, the Skills settings, the Change view and the
File view. A plugin page is written only against the **plugin SDK**,
`@demicodes/plugin-sdk`, so the shell knows no plugin and a plugin knows no
part of the shell beyond the SDK.

This document owns the page side of a plugin: what a page can fill, what the
shell gives it, how it is registered and how its API is versioned.
[Plugins](plugins.md) owns the backend side: the manifest, the contract and the
plugin host.

## A link, end to end

For example, the agent's reply names `src/app.ts` and the user clicks it:

```text
transcript (shell)    the link names a Host file: open the `file` intent
                      with { path: "/home/ada/work/src/app.ts" }
work panel (shell)    the enabled kind that opens `file` is the `file` kind of
                      the `file-browser` plugin, and it is pinned: the panel
                      asks the kind for the pinned tab's next data, selects
                      that tab and opens the panel
`file` kind (plugin)  its content reads the file through the conversation's
                      files service and shows it, with Back to what it
                      showed before
```

With `file-browser` turned off, no enabled kind opens `file`, so the
transcript shows the path as text and offers nothing to click. Neither the
transcript nor the panel names the `file-browser` plugin.

## Packages

```text
web (the product)                         web-gallery
  HTTP, the sync channel                    fixture services
         \                                    /
          +--> generated registry <----------+
                     |
          plugin-browser  plugin-expose  plugin-skills  plugin-changes  plugin-file-browser
                     |    a plugin's feature UI: its components and its state
                     v
               plugin-sdk     slots, PluginClient, intents, services, public components
                     |
                     v
                  web-ui      the shell's components and the primitives the SDK exposes
```

- **`plugin-sdk`** is the whole API a page may use: the slot types, the
  `PluginClient` with `usePlugin()`, the intents, the services, and the public
  components, which it re-exports from `web-ui`. A page package imports
  `@demicodes/plugin-sdk`, `@demicodes/utils`, Vue, Zod and its own generated
  types, and nothing else of the workspace; the [package boundary check](crates-and-packages.md#boundary-checks)
  enforces it.
- **A page package**, `@demicodes/plugin-<name>` in `packages/plugin-<name>`,
  holds the plugin's feature UI: its components, their state and their
  handlers. The browser's live view, the expose menu with its `page` tab, the
  Skills page, the Change view and the File view live in their plugins'
  packages.
- **`web-ui`** holds the shell's components and the reusable primitives:
  buttons, menus, settings rows, progress, the file tree, file previews, the
  diff, the address bar. It knows no plugin. A primitive a plugin needs joins
  the SDK's components when the plugin first uses it.
- **`web` and `web-gallery`** supply the services, over HTTP and the sync
  channel or over fixtures, and show the pages the registry lists.

## Slots

A page package exports one `PluginPage`, which names its plugin and the slots
it fills:

| Slot | The plugin gives | The shell gives | Used by |
| --- | --- | --- | --- |
| Settings section | A navigation entry in a group of the settings rail, and its page | The overlay store for its dialogs | `skills` |
| Conversation header tool | A component the header shows; it shows nothing while it has nothing to show | The conversation, the name of a device, the services | `expose` |
| Work panel kinds | The kinds of tab it shows, made for one conversation's panel | The conversation, its tabs of the plugin's kinds, the services | `browser`, `expose`, `changes`, `file-browser` |

A slot is filled only while the user has the plugin on, in the order the
backend registers its plugins. The slots are the ones above; a slot joins the
SDK when a feature needs it, not before.

## Work panel kinds

A kind tells the panel what it needs to show its tabs: the mark, a title from
`data`, the content component, the schema of `data`, and whether the strip's
new-tab control creates one ([Work panel](../product/web-application.md#work-panel)).
Two more fields make a kind able to replace what used to be fixed parts of
the panel:

- **`pinned`.** A pinned kind has exactly one tab in every conversation's
  panel, ahead of the user's tabs. It is never created, closed or saved; its
  `data` lives in the page's memory for the page's lifetime, starting from
  the kind's initial data. Its id is the kind's id, which the saved selection
  may name. The Change view (`change`) and the File view (`file`) are the
  pinned kinds of `changes` and `file-browser`.
- **`picked`.** What a tab shows next when its user picks it in the strip,
  even while it is selected. The Change view returns to Uncommitted this way
  ([Delivery to the conversation](../execution/edit-tracking.md#delivery-to-the-conversation)).

A plugin makes its kinds for one conversation when the panel opens beside it.
The panel calls the kinds' `refresh()` after each of the conversation's tool
calls finishes, since a tool call may have changed what a kind shows, and
`dispose()` when the panel closes, shows another conversation or the plugin
turns off. A kind that must read again when the page is shown again, as the
browser's tab list and the Change view's counts must, watches the page's
visibility itself. A kind's id is unique across plugins; the panel refuses a
duplicate when it makes the kinds.

## Intents

An intent is a request to show something, named by what it shows rather than
by who shows it. The SDK defines each intent and its payload:

| Intent | Payload | Opened by the shell from |
| --- | --- | --- |
| `file` | `{ path }`, an absolute path on the conversation's Host | A file a message names ([Files named in messages](../product/file-previews.md#files-named-in-messages)), an attachment's capsule |
| `edit` | One call's edit of one file, as the transcript's tool block names it | A tool call's file pill, a message edit's selection |

A page declares the intents it opens, each with the pinned kind whose tab
shows it, on its `PluginPage` rather than on the kind, since an intent can
arrive while the panel is closed and no kind is made. To open an intent, the
panel takes the first enabled page that declares it: that kind's pinned tab
takes the data the page returns from the payload and the tab's current data,
and is selected, and the panel opens if it was closed. Whoever shows a control
that opens an intent asks the panel first whether any page opens it, and
shows plain text otherwise. A plugin's components open intents through the
same service as the shell, and open a tab of the plugin's own kinds with its
data; a plugin cannot add a tab of another plugin's kind.

## Services

What the shell gives a page, each an interface of the SDK that `web` supplies
over the backend and the gallery over fixtures:

| Service | Gives |
| --- | --- |
| Plugin client | The plugin's state, its page calls, its user streams and the installs of its packages ([Plugins](plugins.md#the-page)) |
| Conversation files | For a conversation whose Host is known: the file tree, text and raw reads, the working tree's changes and a call's retained edits ([File previews](../product/file-previews.md), [Edit tracking](../execution/edit-tracking.md)) |
| Intents | Opening an intent, and whether any page the user has on opens it |
| Panel | Adding a tab of the plugin's own kinds |
| Settings | Opening a settings section, such as Devices |
| Errors and overlays | Reporting an error the user sees, and the overlay store dialogs open in |

The conversation's files are a product service, not a plugin's: the runner
records every job's edits and the backend serves every read of a Host file,
for whichever page shows them. `changes` and `file-browser` only show them.

## Types

A plugin's types on the page are generated from its Rust types into its
package's `src/generated/`: its page state, its page calls' parameters and
results, and its user streams' messages, such as the live view's frames,
which `plugin-browser` validates. A page never declares a schema by hand for
a shape the plugin's crate defines ([Generated TypeScript](contracts.md#generated-typescript)).

## Registration

A plugin is one id across its parts: its backend crate, which the backend's
composition root links ([The plugin host](plugins.md#the-plugin-host)), its
page package, and the command packages its commands bind. A plugin that only
shows a page, such as `changes`, has a backend crate whose manifest declares
only its id, name and description: it still has a switch in settings, and
the page's slots follow it.

A page package says which plugin it is in its `package.json`,
`"demi": { "plugin": "<id>" }`, and its entry's default export is its
`PluginPage`. `xtask contracts` writes the registry, the static list of
imports that `web` and `web-gallery` show, from the page packages each of
them depends on, in the order the backend registers their plugins; that order
is the order of the slots they fill. Generation fails when a package's id is
not one the backend registers. Two pages that register one kind fail when the
panel makes the kinds, which every specimen and the product's tests do.
Adding a plugin page to the product is adding the dependency; no import is
written by hand, and no import is computed.

## Versions

The SDK's major version is the page API's version. A change that could break
a page written against it, such as a slot, service or component removed or
changed incompatibly, raises the major version; adding one does not. The
pages of this repository always build against the SDK beside them. A page
built elsewhere is checked against the major version it declares, with the
loading below.

## Pages from outside the repository

Open, decided with the TypeScript SDK ([Decisions for the TypeScript SDK](plugins.md#decisions-for-the-typescript-sdk)).
What the design above already keeps possible:

- A page from outside is an ES module built against one SDK major version,
  with Vue and the SDK supplied by the page around it; the backend serves it
  with the plugin.
- Every service passes data only, so a page whose code nobody reviewed can run
  in a sandboxed frame, with its `PluginClient` and services over messages to
  the shell, without changing the page's code.
