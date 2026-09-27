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

The browser uses same-origin cookie authentication. REST carries every
operation, and what a page reads when it needs it, such as a transcript or a
draft. The synchronization channel pushes the state a page shows around its
conversations, as it changes ([Page synchronization](#page-synchronization)).
The conversation WebSocket carries the
[agent frames](../agent/runtime.md#frame-protocol) of one conversation: live
transcript, queue, child-agent, and shell-job events. A live browser view uses
its own [user stream](web-api.md#user-streams) WebSocket, so its pictures never
delay those events.

Every REST answer, synchronization message and conversation frame is checked
against its generated schema before the page uses it. `web`'s API adapters
check each REST response before applying it to state, the synchronization
module checks each message, and `AgentClient` checks every frame it receives,
with the transcript blocks and tool views inside it. [Web API](web-api.md)
defines the HTTP contracts; [Authentication](#authentication) defines session
integration.

Model discovery uses one account-wide cache and shared pending request
([Catalog cache](../providers/models.md#catalog-cache)).
Execution availability is derived separately for each conversation. History
loading does not wait for model discovery.

The conversation store uses the shared `ConversationCache` to retain opened
state, initialization, and runtimes for the signed-in page lifetime. Switching
conversations reuses healthy connections; inactive cached sessions still receive
events. Context changes, archive changes, removal, retry, and account cleanup
invalidate the appropriate entries. Disposing a browser runtime closes its
attachment without stopping the backend task. Reload starts a new browser cache.

A conversation open in several tabs or browsers is live and usable in each.
For example, the user sends a message in one tab: a second tab shows the
message, the reply as it streams and each tool call, and its Stop stops the
turn. Every tab shows the same messages, output of running commands as it comes
([Live output](../agent/runtime.md#live-output)), phase,
queue, pending steers and subagents; every tab can send, steer, queue, stop, edit,
retry, switch the model and write to a running command; and what one tab does
shows in all of them. Opening the conversation in another tab takes nothing
over. Every tab shows the conversation's one draft, and what one tab types
reaches the others moments after it is saved ([Drafts](#drafts)). A turn or
save that fails shows its failure in every tab until the
conversation starts its next action, whichever tab starts it; a refusal of one
tab's own request shows in that tab alone. Each tab has its own socket,
attached to the conversation's one live tree
([Connections and the live tree](../agent/runtime.md#connections-and-the-live-tree)).
The conversation's model settings, its model, thinking effort and service
tier, are one value its record holds: every tab and every device shows it, and
a change made in any of them reaches all
([Sidebar mutations and read state](web-api.md#sidebar-mutations-and-read-state)).
A tab whose socket is lost or taken as broken
([Liveness and reconnection](#liveness-and-reconnection)), or whose tree
another client disposed with `close`, opens the conversation again the same
way and shows the tree as the backend then has it.

### Liveness and reconnection

A page holds WebSockets of three kinds to the backend: the synchronization
channel, a socket for each open conversation, and the stream of each
[live browser view](../browser/live-view.md). All three connect again by one
rule, the first two also tell a quiet socket from a dead one by one rule, and
one module of `web-ui` implements both. For example, a laptop sleeps and its
network drops without a close. Nothing tells the page: its sockets still look
open, and a conversation would go on showing a turn as running. The backend
sends a heartbeat on the channel and on each conversation socket that has
sent nothing else for 30 seconds
([Order and delivery](../agent/runtime.md#order-and-delivery),
[Page synchronization](web-api.md#page-synchronization)), so a socket that
brings nothing for 75 seconds, two and a half heartbeats, is broken: the page
closes it and connects again. A live view's stream has a heartbeat of its
own, by which the page shows a stall
([Delivery](../browser/live-view.md#delivery)).

A socket that closes, is taken as broken, or cannot be made connects again
after a second, then after twice as long each time, up to 30 seconds, each
wait shortened by a random part so that the pages of all users do not return
at once after a restart. The waits start over at a second once the socket
works again: the channel when its snapshot arrives, a conversation when it
opens, a live view when its first `state` arrives. Meanwhile the page keeps
the channel's copy as it was, and a conversation shows that it is connecting.
A conversation socket that is lost before the session answered `open`, as
when the backend restarts while the conversation opens, follows the same
rule: only a session that answers `open` with a refusal, an `error` or a
`rejected`, shows the conversation as failed.

A page that comes back does not wait. Its timers stop while its computer
sleeps, so the 75-second watch counts only the time the page was awake, and a
wait that began before the sleep still has the rest of its time to run. So
when the page becomes visible again or comes back online, the module checks
every socket at once. A channel or conversation socket whose last message
came 75 seconds ago or more by the clock is broken, and one heard from since
is watched for the rest of those 75 seconds by the clock. A closed socket
connects without waiting for the rest of its wait. For example, a laptop
sleeps for an hour with the product open, and its user then opens the lid:
the page shows again, and replaces the channel and each conversation socket
at once rather than after up to 75 seconds more of a watch that did not count
the hour.

### Page synchronization

One module of `web`, the product state, holds the page's copy of the user's
state and is the one place that follows it. It opens the
[synchronization channel](web-api.md#page-synchronization) once the page is
signed in, takes the channel's `snapshot` as its whole copy, and replaces one
part of the copy with each later message. Nothing on the page asks for this
state on a timer, or asks again after a write. For example, a user pins a
conversation in one tab: the other tab moves it among the pinned ones when
the summary and the order arrive on its channel, a few milliseconds after the
pin committed, while it does nothing itself.

Each synced state follows the copy with its own rule:

| State | Follows | Its own rule |
| --- | --- | --- |
| The sidebar: titles, pins, archive, projects and order | Each summary, and `conversation_order` | A title the user typed shows from the moment it is typed until its write is answered |
| Read state | Each summary's `readRevision` and `unread` | An acknowledgement only moves forward |
| Model settings | Each summary's `model` | A change names only the part its user changed ([Sidebar mutations and read state](web-api.md#sidebar-mutations-and-read-state)) |
| Drafts | Each summary's `draftRevision` | A page that shows the conversation reads the draft when the revision is higher than its own ([Drafts](#drafts)) |
| Preferences | `preferences` | A change shows at once, and the part it changed stays as the user set it until its write is answered |
| The account, workspaces, devices, exposes, providers and the Cloud | Their parts | The page shows what the backend holds |

**A page's own writes.** A write's answer carries the state as the write left
it, and the page applies it to its copy at once, through the same module,
unless the channel has brought a value of that part since the page sent the
write. That value may be newer than the answer; if it is older, the value the
write causes is still to come on the channel, so dropping the answer loses
nothing. For example, tab A renames a conversation, and tab B pins it just
after the rename commits. A's channel brings the summary with both changes
before A receives the answer to its rename: A keeps the summary and drops the
answer, which lacks the pin. Writes whose answer carries no state, such as a
reorder, show from the channel alone; a flow that goes on with what its write
made, such as opening a new provider entry's page, waits for the channel to
bring it.

**Connection.** A channel that closes, or brings nothing for 75 seconds,
connects again as [Liveness and reconnection](#liveness-and-reconnection)
says, at once when the page comes back.
The new `snapshot` replaces the whole copy, and each state follows it as it
follows any change. The browser does not say why an upgrade failed, so a page whose channel does not open asks
`GET /api/auth/me`, whose 401 ends the session as any 401 does
([Authentication](#authentication)). Until the first snapshot the page shows
its loading state; a first connection that fails shows the failure with a
retry, which connects at once.

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

The synchronization channel and API answers detect an ended session: a
channel closed with `session_ended`, or an answer of 401, makes the page
release its account-scoped stores and transports and show that the session
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

The backend owns saved conversation state, ordering, preferences, attachments,
and each conversation's draft ([Drafts](#drafts)). Per-user IndexedDB keeps
what the backend does not have yet or does not keep: draft edits the backend
has not confirmed, with the bytes of their files not yet uploaded;
unconfirmed submissions; new conversations, with their model settings, until
their first send writes them to the record; edits of sent messages in
progress; and each conversation's scroll position. A conversation with a
record keeps its model settings only there
([Sidebar mutations and read state](web-api.md#sidebar-mutations-and-read-state)).
The tabs of one browser share this storage, and a page writes a
conversation's record only for a change its own user made, so a tab never
replaces what another tab wrote for a conversation it left alone.
Browser-local preferences
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

### Drafts

For example, a user types "Fix the login" into the composer in a tab on their
laptop and drops `trace.txt` into it. The file starts uploading at once. Half
a second after the typing pauses, the tab saves the draft, and once the upload
is done it saves it again with the file. A second tab and the user's phone,
which show the same conversation, show the text and the file's capsule moments
after each save, when their synchronization channels bring the conversation's
new draft revision, and the phone can send the message with the file. Once
the backend accepts the message, from whichever page sent it, the composer is
empty everywhere.

The backend keeps the draft, its revision, and the version a save replaced
([Conversation drafts](web-api.md#conversation-drafts)). A page:

- **Saves** half a second after typing pauses, and when it is hidden or
  closed. A save carries the revision the page's text was built on. A page
  sends one request about a conversation's draft at a time, each after the
  answer to the one before, so its own saves never replace each other unseen.
  A save that does not reach the backend is tried again a few seconds later;
  a refused one is reported, and the next change tries again.
- **Stages files** by uploading each one as it is added. A file joins the
  saved draft once its upload is done; until then the saved text leaves out
  its mark, so other pages show the text without that capsule. A file on a
  paired device joins at once.
- **Follows** the draft: it reads it when it opens the conversation, and again
  when its synchronization channel brings the conversation's summary with a
  higher `draftRevision` ([Page synchronization](#page-synchronization)).
  Its own save, restore or dismissal shows at once, from the answer.
- **Keeps what its user types.** A page has an unsaved change from the moment
  its user edits the draft until the backend confirms a save that holds the
  edit. Without one, the composer shows each newer draft it reads. With one,
  the composer keeps its text and never puts another page's draft in its
  place: its save, due within half a second, is based on the older revision,
  so it wins, and the draft it replaces becomes the replaced version.
- **Offers the replaced version.** While the draft has one, every composer
  that shows the conversation offers it by its first line, to restore with
  one click or to dismiss. Restore exchanges it with the draft, so the text
  it displaces is offered in its place and nothing is lost.
- **Clears the draft on send.** A sent message leaves the composer at once and
  shows as the pending submission, but the saved draft stays until the
  backend accepts the message; until then the emptied composer is the page's
  unsaved change. The page then saves an empty draft, based on the revision
  the message was built on, so a change another page made meanwhile becomes
  the replaced version. A page that closes before it sees the message
  accepted clears the draft when it next confirms the submission, which
  IndexedDB keeps.

Two places that edit the draft at once, two tabs typing together or a device
that edited offline, follow one rule: the save that reaches the backend last
wins, and the version it replaced is offered for restore
([Conversation drafts](web-api.md#conversation-drafts) has an example).

**Offline.** Every change is written to IndexedDB as the user makes it, with
the revision it was based on and the bytes of files not yet uploaded, and it
stays there until the backend confirms a save that holds it. A page that
cannot reach the backend goes on writing that record. When it can again, it
uploads the waiting files and saves, and its save wins over what other places
saved meanwhile, which is then offered for restore. An IndexedDB write still
under way when the page closes can be lost, so a closing page also writes each
unconfirmed composer's text to local storage, which the browser writes at
once, and the next page of the conversation takes that text over the record.
A page that opens with such a record shows it instead of the backend's draft
and saves it, unless the backend's draft is that text already, as when the
save on closing arrived; it then only takes the backend's revision. The tabs
of one browser share the record, so the last edit written is the one kept, as
the last save is.

**A new conversation** has only its local UUID until the first send creates
its backend record. Its draft stays in IndexedDB, in the browser it was
started in: other tabs of that browser list it when they load, and other
devices do not see it. The first send empties the draft, and every later
draft of the conversation lives in the backend.

An edit of a sent message ([Message editing](../agent/message-editing.md)) is
not the draft: it stays in the browser that opened it.

## Development and checks

Start the backend executable, `demi-backend`, which listens on port 3271 by
default ([Development backend](../backend/backend.md#development-backend) gives
the whole launch), then run the root package script `web:dev` for the product
at `http://127.0.0.1:18934`. Vite forwards `/api` HTTP and WebSocket requests
to the backend; set `DEMI_BACKEND_URL` when the backend runs at another
address. The script runs Vite under Node, so development needs a system Node,
the one part of the toolchain that does: under Bun, which runs the
repository's other scripts (`bunfig.toml`), Vite's proxy cannot forward a
WebSocket, since Bun's `node:http` client never reports the backend's
upgrade, and the conversation would never connect. Create the first account
through the setup API (`POST /api/setup`).
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
