# Web architecture

Demi's browser application is a Vue and TypeScript SPA, built with Vite and
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
web                                 web-gallery
account state, routing,             fixture data and
backend adapters                    preview handlers
        |                                  |
        +------------> web-ui <------------+
                         |
                         v
                    agent-client     AgentClient, transport, patch application
                         |
                         v
                      protocol       generated schemas and types
```

- `web-ui` owns reusable components and browser interaction.
- `web` supplies product data, state, routing, and API handlers.
- `web-gallery` supplies fixtures and handlers to demonstrate the same components.
- `@demicodes/agent-client` is the client side of a conversation stream:
  `AgentClient` and its waiters, the WebSocket transport, and the one function
  that applies transcript patches.
- `@demicodes/protocol` is generated. It holds the schemas and types of the
  Rust contracts the page reads, such as agent frames, transcript blocks,
  message content, tool views and the live view's messages, and the file-type
  table with its lookup.
- `@demicodes/utils` holds small helpers the browser packages share.

The product and gallery do not import each other. The browser knows the HTTP
API and the streams only through types generated from the Rust types that
define them: `@demicodes/protocol` for the conversation stream and the live
view, and REST types generated into `web` for the HTTP API
([Generated TypeScript](../architecture/contracts.md#generated-typescript)).
No browser package writes its own schema for a shape those contracts define.
Where the page and the backend could each compute the same fact, one side owns
it ([Logic the browser and backend share](../architecture/contracts.md#logic-the-browser-and-backend-share)):
the backend reads an upload's media type and snippet, and `web-ui` derives a
queued message's summary from its content.
[Crates and packages](../architecture/crates-and-packages.md#typescript-packages)
owns the package registry and the dependency graph. Shared UI changes land in
both consumers in the same checkpoint.

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
offers to try again ([Live browser view](../browser/live-view.md#a-browser-tab-in-the-panel)).

A tab is removed only by its user. A tab whose `data` does not fit its kind's
schema, or whose kind the page does not know, stays in the strip and its
content says that it cannot be shown; the page never repairs or drops it.

The kinds:

| Kind | Shows | Created by |
| --- | --- | --- |
| `browser` | One tab of the [conversation browser](../browser/browser.md), live; [Live browser view](../browser/live-view.md) owns its content | The strip's new-tab control; the agent's `open`, which the kind adds as a tab |
| `page` | A page in the user's own browser, in a sandboxed iframe | Only an [expose](../execution/expose.md#product-surface), on its URL |

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
conversation, project, device, provider, and preference data. The conversation
WebSocket carries the [agent frames](../agent/runtime.md#frame-protocol) of one
conversation: live transcript, queue, child-agent, and shell-job events. A live
browser view uses its own [user stream](web-api.md#user-streams) WebSocket, so
its pictures never delay those events.

Every REST answer and conversation frame is checked against its generated
schema before the page uses it. `web`'s API adapters check each REST response
before applying it to state, and `AgentClient` checks every frame it receives,
with the transcript blocks and tool views inside it. [Web API](web-api.md)
defines the HTTP contracts; [Authentication](#authentication) defines session
integration.

Account state uses conditional snapshots, refreshed periodically and when the
page becomes visible. Model discovery uses one account-wide cache and shared
pending request ([Catalog cache](../providers/models.md#catalog-cache)).
Execution availability is derived separately for each conversation. History
loading does not wait for model discovery.

The conversation store uses the shared `ConversationCache` to retain opened
state, initialization, and runtimes for the signed-in page lifetime. Switching
conversations reuses healthy connections; inactive cached sessions still receive
events. Context changes, archive changes, removal, retry, and account cleanup
invalidate the appropriate entries. Disposing a browser runtime closes its
attachment without stopping the backend task. Reload starts a new browser cache.

## Authentication

The page holds one session state: checking, signed out, or signed in with an
account. Startup checks `GET /api/auth/me` before the initial route mounts, and
the account in its answer is validated before it enters the state. Later route
changes use the current state, so opening a local conversation does not wait
for an authentication request. Without a session, the page goes to `/login`.

The session is the backend's HttpOnly cookie
([Authentication and ownership](../backend/backend.md#authentication-and-ownership)),
which accompanies every same-origin request; JavaScript never receives a
session token. Passwords stay in the form and its request and are cleared
after login.

The sign-in form is a `web-ui` component that owns its busy and error states.
`web` supplies its request handler; the gallery shows the same component in
fixture phases and never calls the backend. `POST /api/auth/login` sends the
email and password. Backend errors, rate limiting included, appear on the
form. Unmounting the form aborts its pending request, and a late answer cannot
change the session state. The login request times out after 60 seconds, as
ordinary API requests do.

Account state polling and API answers detect an expired session: the page
releases its account-scoped stores and transports and shows that the session
ended. `POST /api/auth/logout` must succeed, or report a session that has
already ended, before the page reloads to release account-scoped stores and
drafts. A logout that fails leaves the account signed in and shows an error.

Nickname, email and password changes use the
[account API](web-api.md#account-api). Closing the email or password dialog
leaves a submitted request running; reopening it restores the current phase
and inputs, and a failure while it is closed also shows a toast. Completion
clears the temporary credentials. Signing out or leaving the signed-in page
aborts account requests. The nickname saves itself as it is edited, reports
its save state, and keeps failed input for a retry.

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
[Message editing](../agent/message-editing.md) and
[Conversation Fork](../agent/conversation-fork.md). The page reads whether a
message can be edited from its block type alone, as the backend does
([Editable blocks](../agent/message-editing.md#editable-blocks)).

Product adapters connect shared file interfaces to device filesystem APIs,
the working-tree change routes, the raw file routes behind
[file previews](file-previews.md), and uploads; the live browser view's source
to the user stream route; and shared account interfaces to provider, pairing,
and Cloud APIs. The main composer and the edit composer add a file through the
same upload adapter ([Attachments](product.md#attachments)); no message
carries a file's bytes. The product reports the time zone and languages of the
user's browser to the user's preferences when they change.
The work panel keeps one file selection and one change selection per
conversation. Closing the selected tab selects its nearest remaining
predecessor, then the first remaining tab, then Change.
The change summary comes from the uncommitted working-tree source, refreshed
while the panel is visible, independently of which section is selected.
Historical edit selection follows
[Edit tracking](../execution/edit-tracking.md#delivery-to-the-conversation).

Gallery adapters use fixtures. Submitted operations and requests belong to the
appropriate conversation or account lifetime and are released on its cleanup.
A fixture demonstrates interface behavior; it does not establish persistence,
authorization, native installation, or Cloud recovery.

## Development and checks

Start the backend executable, `demi-backend`, which listens on port 3271 by
default ([Development backend](../backend/backend.md#development-backend) gives
the whole launch), then run the root package script `web:dev` for the product
at `http://127.0.0.1:18934`. Vite forwards `/api` HTTP and WebSocket requests
to the backend; set `DEMI_BACKEND_URL` when the backend runs at another
address. Create the first account through the setup API (`POST /api/setup`).
The script `web:build` writes `packages/web/dist`, which the backend serves when
`DEMI_WEB_DIRECTORY` names it
([Serving the browser build](web-api.md#serving-the-browser-build)). The
script `web:gallery` runs the component catalog independently of credentials
and model services. Generated contract files are not committed; the scripts
that need them generate them first
([Generated TypeScript](../architecture/contracts.md#generated-typescript)).

Changes require the affected browser package typechecks, appropriate mocked
integration tests, and verification in product and gallery. Tests use
disposable accounts and captured mail, and never call real models. The
browser-contract suite in `web` runs the product's API client and
`AgentClient` against the backend executable
([Browser-contract suite](../delivery/scenarios.md#browser-contract-suite)).
Backend acceptance and deployment requirements belong to
[Delivery and acceptance](../delivery/roadmap.md).
