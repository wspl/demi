# Web architecture

Demi's browser application is a Vue 3 and TypeScript SPA, built with Vite and
Tailwind. vue-router owns navigation; Pinia holds application state.
`web/main.ts` composes the router and account-scoped stores. A user message is
a tiptap (ProseMirror) editor: editable in the composer, read-only in the
conversation, so both show it one way
([Writing a message](product.md#writing-a-message)). `web-ui` reads and writes
the message's Markdown itself rather than through tiptap's Markdown extension,
which backslash-escapes every `_` and writes `<` as `&lt;` in what the model
would read.

## Package responsibilities

```text
web: account state, routing, backend adapters --------+
                                                     v
                                                   web-ui
                                                     ^
web-gallery: fixture data and preview handlers -------+
```

- `web-ui` owns reusable components and browser interaction.
- `web` supplies product data, state, routing, and API handlers.
- `web-gallery` supplies fixtures and handlers to demonstrate the same components.

The product and gallery do not import each other. Neither imports backend
implementations, execution hosts, or concrete model providers.
[Package boundaries](../package-boundaries.md) defines the dependency contract.
Shared UI changes land in both consumers in the same checkpoint.

The gallery is the reference for components, appearance, layout, and interaction
examples. Those details are not duplicated in design documents. This document
covers only the browser's technology and architectural boundaries.

## Work panel

The work panel shows one thing at a time: a fixed view, or a tab.

```text
selection   'change' | 'file' | <tab id>
tabs        [{ id, kind, data }], in the user's order
```

- **Fixed views.** Change and File belong to the panel itself. They are not
  tabs: they are never created, closed, listed or saved. They only compete
  with the tabs for the selection.
- **Tabs.** A tab is a saved fact: a tab of this kind stands here, with this
  `data`. It has no status, no error and no stored title. Whether its content
  is loading, disconnected or refused is the content's own runtime state, shown
  inside the tab's content and never written to the tab.
- **Kinds.** A kind registers with `web-ui` what the panel needs to show its
  tabs: the mark, the title derived from `data`, the content component, a
  schema for `data`, and whether the user can create one from the strip. The
  panel and the tab state know nothing else about a kind. What protocol,
  stream or route a tab's content uses is the kind's own business, behind its
  content component. A new kind, such as a streamed window of the Host, is a
  new registration and changes neither the panel nor the tab state.

The tab state is ordinary state with `add`, `update`, `remove`, `move` and
`select`. Every change applies to the page first and is then saved
([Work panel state](web-api.md#work-panel-state)); a save that fails is
reported as any failed save is and retried with the next change. A kind's
content reaches its own tab only through `update` of its `data`.

For example, the user presses the strip's new-tab control. The panel adds
`{ id, kind: 'browser', data: { url: 'about:blank' } }`, selects it and saves.
The tab is there at once, whatever the Host is doing. Its content then asks
for a tab in the conversation browser, writes that tab's id into `data`, and
shows the page live; while that takes time or fails, the content says so and
offers to try again ([Live browser view](browser-live-view.md#a-browser-tab-in-the-panel)).

A tab is removed only by its user. A tab whose `data` does not fit its kind's
schema, or whose kind the page does not know, stays in the strip and its
content says that it cannot be shown; the page never repairs or drops it.

The kinds:

| Kind | Shows | Created by |
| --- | --- | --- |
| `browser` | One tab of the [conversation browser](browser.md), live; [Live browser view](browser-live-view.md) owns its content | The strip's new-tab control; the agent's `open`, which the kind adds as a tab |
| `page` | A page in the user's own browser, in a sandboxed iframe | Only an [expose](expose.md#product-surface), on its URL |

A `page` tab loads the `http` or `https` address the user submits in its
address bar, which it shares with the `browser` kind.
The frame may run scripts, submit forms and open popups, which land in
ordinary browser tabs; it cannot navigate the product page. The parent sees
nothing of a cross-origin page, so Back and Forward stay unavailable, Refresh
reloads the tab's URL, and a control opens that URL in an ordinary browser
tab for pages that refuse framing.

## Backend communication

A request asks for something and an answer says what happened. Every control
operation is therefore an ordinary HTTP request: it has one answer, a status
and an error the caller can show, it appears in the network panel and the
backend's log, and it can be retried. A WebSocket carries only what a request
cannot: bytes that flow for as long as the user watches, such as pictures and
the input that must stay in order with them, and events the backend pushes as
they happen. An operation is never a message on a stream that some later
message may or may not answer: a lost answer then looks exactly like a slow
one, and nothing records that it failed.

The browser uses same-origin cookie authentication. REST supplies account,
conversation, project, device, provider, and preference data. The agent WebSocket
supplies live transcript, queue, child-agent, and shell-job events. A live
browser view uses its own [user stream](web-api.md#user-streams) WebSocket, so
its pictures never delay those events. Browser API
adapters validate REST responses before applying them to state. Agent frames,
transcript blocks, and tool views are validated against the session contract's
own schemas: the agent client checks every frame it receives, and `web-ui`
re-exports those schemas (`transport/protocol.ts`) so the product describes the
wire once, where it is produced.
[Web API](web-api.md) defines HTTP contracts;
[Authentication](../web-authentication.md) defines session integration.

Account state uses conditional snapshots, refreshed periodically and when the
page becomes visible. Model discovery uses one account-wide cache and shared
pending request. Execution availability is derived separately for each
conversation. History loading does not wait for model discovery.

The conversation store uses the shared `ConversationCache` to retain opened
state, initialization, and runtimes for the signed-in page lifetime. Switching
conversations reuses healthy connections; inactive cached sessions still receive
events. Context changes, archive changes, removal, retry, and account cleanup
invalidate the appropriate entries. Disposing a browser runtime closes its
attachment without stopping the backend task. Reload starts a new browser cache.

## Persistence and adapters

The backend owns saved conversation state, ordering, preferences, and attachments.
Per-user IndexedDB retains local drafts, pending edits, unconfirmed submissions,
and attachment bytes. A draft is its Markdown, with a mark where each staged
file's capsule sits, and those files in mark order. Browser-local preferences
hold presentation-only choices. Work-panel width and per-conversation open/closed state use the same account-scoped
local preferences. Refreshing restores whether the panel was open; a conversation
without a saved choice starts closed. The panel's tabs and selection are saved
with the conversation ([Work panel](#work-panel)); the File and Change views'
own selections and address drafts remain in memory for the page lifetime. Switching accounts uses that account's
saved choices. These stores do not replace backend ownership or authorization.

New conversations begin with a local UUID. The first send creates the backend
record using that ID. Submissions retain their message ID until admission is
confirmed, so retry can reconcile history and queue state before resending.
Editing and Fork use their backend operation contracts; see
[Message editing](../message-editing.md) and [Conversation Fork](../conversation-fork.md).

Product adapters connect shared file interfaces to device filesystem APIs,
the working-tree change routes, the raw file routes behind
[file previews](file-previews.md), and uploads; the live browser view's source
to the user stream route; and shared account interfaces to provider, pairing,
and Cloud APIs. The product reports the time zone and languages of the user's
browser to the user's preferences when they change.
The work panel keeps one file selection and one change selection per
conversation. Closing the selected tab selects its nearest remaining
predecessor, then the first remaining tab, then Change.
The change summary comes from the uncommitted working-tree source, refreshed
while the panel is visible, independently of which section is selected.
Historical edit selection follows [Edit tracking](edit-tracking.md#delivery-to-the-conversation).

Gallery adapters use fixtures. Submitted operations and requests belong to the
appropriate conversation or account lifetime and are released on its cleanup.
A fixture demonstrates interface behavior; it does not establish persistence,
authorization, native installation, or Cloud recovery.

## Development and checks

Run the backend, then `bun run web:dev` for the product at
`http://127.0.0.1:18934`. `bun run web:build` writes `packages/web/dist`.
`bun run web:gallery` runs the component catalog independently of credentials and
model services.

Changes require the affected browser package typechecks, appropriate mocked
integration tests, and verification in product and gallery. Tests never call real
models. Backend acceptance and deployment requirements belong to
[Delivery and acceptance](roadmap.md).
