# Web architecture

Demi's browser application is a Vue 3 and TypeScript SPA, built with Vite and
Tailwind. vue-router owns navigation; Pinia holds application state.
`web/main.ts` composes the router and account-scoped stores.

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

[Conversation browser](browser.md#purpose) currently covers agent commands only.
The work panel includes a browser placeholder owned by `web-ui`; its local tabs
and address drafts do not connect to the Host browser. Page rendering, navigation
and streaming transport remain deferred. Explicit screenshots use existing
command media handling; live browser interaction requires a separate design.

## Backend communication

The browser uses same-origin cookie authentication. REST supplies account,
conversation, project, device, provider, and preference data. The agent WebSocket
supplies live transcript, queue, child-agent, and shell-job events. Browser API
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
and attachment bytes. Browser-local preferences hold presentation-only choices.
Work-panel width and per-conversation open/closed state use the same account-scoped
local preferences. Refreshing restores whether the panel was open; a conversation
without a saved choice starts closed. Tab selections and browser address drafts
remain in memory for the page lifetime. Switching accounts uses that account's
saved choices. These stores do not replace backend ownership or authorization.

New conversations begin with a local UUID. The first send creates the backend
record using that ID. Submissions retain their message ID until admission is
confirmed, so retry can reconcile history and queue state before resending.
Editing and Fork use their backend operation contracts; see
[Message editing](../message-editing.md) and [Conversation Fork](../conversation-fork.md).

Product adapters connect shared file interfaces to device filesystem APIs,
the working-tree change routes, and uploads, and shared account interfaces to
provider, pairing, and Cloud APIs.
The work panel keeps one file selection, one change selection and local browser
placeholder tabs per conversation. Change and File are fixed view selections; browser tabs
can be added and closed. One active tab selects the
view. Closing the active browser tab selects its nearest remaining predecessor,
or the first remaining tab. Browser address drafts belong to their tabs.
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
