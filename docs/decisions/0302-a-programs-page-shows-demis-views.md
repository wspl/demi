---
status: proposed
---

# A program's page shows a conversation and its work panel with the web app's own views

## Problem

A program that runs Demi's agents, such as a bot's console, wants to show
a conversation as Demi shows it: the transcript as it streams, the
composer, the permission cards, and the work panel with the files, the
changes and the browser's live view. Those views are the web app's: they
read and write through the web app's client at `/api`, behind a Demi
session, inside the web app's shell. A program would rebuild them, and its
copy would fall behind every change to Demi's.

## Decision

`@demicodes/web-components` defines two custom elements, built from the
web app's own views, which talk to a core through the path where the
program forwards the core's routes (0301's `core.fetch` and
`core.accept`):

```html
<script type="module" src="/assets/demi-web-components.js"></script>
<demi-conversation api="/demi/api" conversation="0193f1c2-..."></demi-conversation>
<demi-work-panel api="/demi/api" conversation="0193f1c2-..."></demi-work-panel>
```

```text
browser                          the program's server          demi-core
<demi-conversation api=/demi/api>
  GET /demi/api/conversations/x ---> its own sign-in check
                                     core.fetch(request) -------> core-api
  WS  /demi/api/connect ----------> core.accept(socket) -------> lanes
```

- `<demi-conversation>` is the web app's conversation view
  (`ChatPage.vue` over `ChatSession.vue`): the transcript, the composer,
  the dock and its cards.
- `<demi-work-panel>` is its work panel (`WorkPane.vue`): the
  conversation's tabs, of the kinds the plugins' pages give, such as files,
  changes and the browser's live view.
- The program checks who may see what before it forwards; the elements
  carry no Demi session.

### One set of views

The elements are a second entry of the web app's build, not a copy:

- The web client's base path, `/api`, written once today in `apiUrl`
  (`api/client.ts`), which the requests and the page connection use,
  becomes a value the app sets to `/api` and an element to its `api`
  attribute.
- What the views ask of the app's shell becomes an event of the element:
  a link to another conversation, to a setting or to a file outside the
  conversation is a `navigate` event, which the program handles. The web
  app keeps doing what it does now.
- The elements read the core's product state through the same
  synchronization lane, and register the same plugins' pages.

The elements are Vue's own custom elements (`defineCustomElement`), with
Vue and the views inside, so a page in any framework, or none, uses them.

## Built on

| What the design uses | Where it is today | New |
| --- | --- | --- |
| The conversation view | `web/src/conversation/ChatPage.vue`, `web-ui/src/agent/ChatSession.vue` | in a custom element |
| The work panel | `web/src/conversation/WorkPane.vue`, the plugins' panel kinds | in a custom element |
| The client's base path | `/api` in `apiUrl` (`api/client.ts`) | an attribute |
| Navigation | the web app's router, in `ChatPage.vue`, `navigation.ts`, `work.ts` | an event outside the app |
| The routes and sockets | `core.fetch`, `core.accept` (0301) | |
| A view rendered without the backend | the gallery's specimens of `ChatSession` | |

## Spike

Not run yet, and run before this record is accepted:

- **The elements in another app.** A page of a Hono server shows
  `<demi-conversation>` and `<demi-work-panel>` over a core the server
  forwards to: a reply streams, the composer sends, a permission card is
  answered, a menu and a dialog open with their styles, and the browser's
  live view plays. `web-ui` places menus and dialogs with `Teleport` 46
  times, and Vue's custom elements and `Teleport` have open issues with a
  shadow root.

## Prior art

- **CopilotKit**: its React provider takes a `runtimeUrl`, such as
  `/api/copilotkit`, where the developer's server mounts the runtime's
  handler; the components talk only to that path.
- **Vercel AI SDK UI**: `useChat({ api: '/api/chat' })` talks to a route
  the developer writes.
- **Vue's `defineCustomElement`**: a Vue component as a standard custom
  element, used by any page.

Demi departs in shipping custom elements rather than components of one
framework, as the web app is Vue and a program's page may be anything.

## Alternatives

- **The views' components with props, as the gallery uses them.** A
  program would wire 36 props of `ChatSession` and the services the web
  app provides around it: the web app's store, written again.
- **Components for React and Vue.** Two more builds of the same views for
  two frameworks, and none for the others.
- **The web app in a frame.** Its shell, sign-in and routes inside the
  program's page, with no way for the program to place the work panel
  apart or to handle navigation.

## Open questions

- None.
