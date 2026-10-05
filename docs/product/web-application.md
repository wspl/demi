# Web architecture

Demi's web app is a Vue and TypeScript SPA, built with Vite and
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
backend adapters, the               preview handlers, the
plugins' services over HTTP         plugins' fixture services
        |          \              /       |
        |           plugin-<name>         |    a plugin's page: its feature UI
        |                 |               |
        |            plugin-sdk           |    the page API
        |                 |               |
        +------------> web-ui <-----------+
                         |
                         v
                    conversation-client     ConversationClient, transport, patch application
                         |
                         v
                      protocol       generated schemas and types
```

- `web-ui` owns the shell's components, the reusable primitives and UI
  interaction. It knows no plugin.
- `web` supplies product data, state, routing, and API handlers.
- `web-gallery` supplies fixtures and handlers to demonstrate the same components.
- `@demicodes/conversation-client` is the client side of a conversation stream:
  `ConversationClient` and its waiters, the WebSocket transport, and the one function
  that applies transcript patches.
- `@demicodes/protocol` is generated. It holds the schemas and types of the
  Rust contracts the page reads, such as agent frames, transcript blocks,
  message content and tool views, and the file-type table with its lookup.
- `@demicodes/utils` holds small helpers the web app's packages share.
- `@demicodes/plugin-sdk` is the page API ([Plugin pages](../architecture/plugin-pages.md)):
  `definePage`, the page context `usePage()`, intents, the conversation files
  service and the plugin kit.
- `@demicodes/plugin-<name>` is a [plugin's page](../architecture/plugin-pages.md):
  the plugin's feature UI and the slots it fills, written only against
  `plugin-sdk`. `web` and the gallery show the packages of the generated
  registry; `web` supplies the page context over the
  [page call route](web-api.md#plugin-calls) and the backend's other routes,
  the gallery over each specimen's fixture state.

The product and gallery do not import each other. The web app knows the HTTP
API and the streams only through types generated from the Rust types that
define them: `@demicodes/protocol` for the conversation stream and the live
view, REST types generated into `web` for the HTTP API, and each plugin's page
types generated into its package
([Generated TypeScript](../architecture/contracts.md#generated-typescript)).
No web app package writes its own schema for a shape those contracts define.
Where the page and the backend could each compute the same fact, one side owns
it ([Logic the web app and backend share](../architecture/contracts.md#logic-the-web-app-and-backend-share)):
the backend reads an upload's media type and snippet, and `web-ui` derives a
queued message's summary from its content.
[Crates and packages](../architecture/crates-and-packages.md#typescript-packages)
owns the package registry and the dependency graph. Shared UI changes land in
both consumers in the same checkpoint.

The gallery is the reference for components, appearance, layout, and interaction
examples. Those details are not duplicated in design documents. This document
covers only the web app's technology and architectural boundaries.

## Responding to the user

The page shows what an action of the user does the moment the user acts, in
the form it will keep, and lets the backend and the Host catch up behind it.
For example, the user presses the work panel's new-tab control: the browser tab
stands in the strip at once, selected, with its address bar on `about:blank`
and a blank page, which is what a new tab shows. The tab in the conversation
browser opens behind it, and its first picture replaces the blank page without
anything else changing. If the user types an address and presses Enter before
that tab exists, the address bar shows the address at once, the page shows that
it loads, and the tab opens on the address the user last asked for.

- **What the page knows, it shows.** A result the page can tell beforehand,
  such as a new tab, a closed tab, a moved tab or a typed address, is shown
  at once and never as a loading state.
- **What it cannot know, it shows as loading, in its place.** A page that
  loads, a file being read or a list not yet received is unknown until it
  arrives. Its loading shows where the result will stand, such as a progress
  line over the page a tab still shows, and never replaces what is already
  known: a tab keeps its last picture while the next page loads or while the
  view reconnects.
- **Later wishes wait their turn.** An action that needs an earlier one to
  land first, such as navigating a tab that is still opening, is kept as the
  user's latest wish and carried out once it can be; a wish the user replaced
  before then is never carried out.
- **Only a failure interrupts.** When the backend or the Host refuses what
  the user did, the page says so where it happened and offers to try again,
  and shows what is really there again.

## Work panel

The work panel shows one tab at a time. Its frame belongs to the shell; every
tab it shows is of a kind a plugin registers ([Work panel kinds](../architecture/plugin-pages.md#work-panel-kinds)).

```text
pinned      one tab of each pinned kind, in the page's memory: change, file
tabs        [{ id, kind, data }], in order, the backend's, with a revision
history     the tabs' and pinned kinds' ids this page selected, newest last
```

- **Pinned tabs.** A pinned kind, such as the Change view's `change` and the
  File view's `file`, has one tab in every conversation's panel, ahead of the
  user's tabs. It is never created, closed, listed or saved; its `data`, such
  as the file it shows and what Back returns to, stays in memory for the
  page's lifetime. Its id is its kind's id.
- **Tabs.** A tab is a fact the backend keeps for the conversation: a tab of
  this kind stands here, with this `data`. The user, from any page, and the
  plugin that owns the kind change the tabs, and the backend orders every
  change ([Work panel state](web-api.md#work-panel-state)), so every page
  shows the same tabs. A tab's `data` is what its kind needs to show it
  again; whether its content is loading or disconnected right now is the
  content's own runtime state and is not written to the tab.
- **Kinds.** A kind registers what the panel needs to show its tabs: the
  mark, the title derived from `data`, the content component, a schema for
  `data`, whether the user can create one from the strip, whether it is
  pinned, and the intents it opens. The panel and the tab state know nothing
  else about a kind. What protocol, stream or route a tab's content uses is
  the kind's own business, behind its content component. A new kind is a new
  registration and changes neither the panel nor the tab state.
- **Selection.** Each page keeps its own selection history for each
  conversation, in the account's local preferences beside whether the panel
  is open, so two pages never take the selection from each other. Selecting
  a tab moves it to the newest end; a closed tab leaves the history; it keeps
  at most 100 entries, more than a panel has tabs. The panel shows the newest
  entry it still shows. So closing the shown tab shows the tab selected
  before it: a user who went from File to tab A to tab B and closes B sees A,
  and closing A then shows File. The same holds for a tab another page
  closed or a plugin turned off. When no entry is left, the panel shows its
  first tab, the Change view.
- **Before the first send.** A new conversation has no backend record yet,
  and so no working directory on a Host
  ([Persistence and adapters](#persistence-and-adapters)). Its panel binds no
  plugin page and neither reads nor saves the panel: it shows its frame,
  which says that files and changes appear once the first message is sent.
  When the first send creates the record, the pinned tabs appear and the
  panel's tabs are read, as for any conversation.

The page changes the tabs with four operations: `create`, `update`, which
sets some of a tab's `data` fields and leaves the others, `remove` and
`move`. The page shows each change at once
([Responding to the user](#responding-to-the-user)) and sends it to the
backend, one at a time and in order. What the page shows is the panel it last
read with the changes not yet in it applied on top: a change leaves that list
once the page has read a panel whose revision is at least the one the
change's answer named. A panel read after the page removed a tab therefore
never brings the tab back, and a tab the page created keeps its id, its place
and its content when the backend's panel takes it over. A change the backend
refuses leaves the list, the page shows the panel as the backend has it, and
the refusal is reported as any failed write is. A kind's content reaches its
own tab only through `update` of its `data`.

For example, the user presses the strip's new-tab control. The page creates
`{ id, kind: 'browser', data: { url: 'about:blank' } }` with a new id, selects
it and sends the change. The tab is there at once, whatever the Host is doing.
The browser plugin, told by the backend that its user created the tab, opens a
tab in the conversation browser and writes that tab's id into `data`; the
content shows the page live from then on
([Live browser view](../browser/live-view.md#a-browser-tab-in-the-panel)).

A tab whose `data` does not fit its kind's schema, or whose kind the page does
not know, stays in the strip and its content says that it cannot be shown;
the page never repairs or drops it.

The kinds:

| Kind | Plugin | Shows | Created by |
| --- | --- | --- | --- |
| `change` | `changes` | Pinned: the conversation's changes, in Uncommitted and Conversation mode ([Changes](file-previews.md#changes)) | The `edit` intent |
| `file` | `file-browser` | Pinned: one file of the conversation's Host, with its tree ([File previews](file-previews.md)) | The `file` intent |
| `browser` | `browser` | One tab of the [conversation browser](../browser/browser.md), live; [Live browser view](../browser/live-view.md) owns its content | The strip's new-tab control; the plugin, for a tab the agent or a page opened |
| `page` | `expose` | A page in the user's own browser, in a sandboxed iframe | Only an [expose](../execution/expose.md#product-surface), on its URL |

A `page` tab loads the `http` or `https` address the user submits in its
address bar, the same primitive the `browser` kind's address bar is.
The frame may run scripts, submit forms and open popups, which land in
ordinary tabs of the user's browser; it cannot navigate the product page. The
parent sees nothing of a cross-origin page, so Back and Forward stay
unavailable, Refresh reloads the tab's URL, and a control opens that URL in an
ordinary tab of the user's browser for pages that refuse framing.

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

The web app uses same-origin cookie authentication. REST carries every
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
module checks each message, and `ConversationClient` checks every frame it receives,
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
invalidate the appropriate entries. Disposing a runtime in the page closes
its attachment without stopping the backend task. Reloading the page starts a
new cache.

A conversation open in several tabs, or in several web browsers, is live and
usable in each.
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
[live browser view](../browser/live-view.md). All three tell a quiet socket
from a dead one by one rule and connect again by another, and one module of
`web-ui` implements both. For example, a laptop sleeps and its network drops
without a close. Nothing tells the page: its sockets still look open, a
conversation would go on showing a turn as running, and a live view would go
on showing its stream as stalled. The far end of each socket sends a
heartbeat once it has sent nothing else for a while: the backend on the
channel and on each conversation socket after 30 seconds
([Order and delivery](../agent/runtime.md#order-and-delivery),
[Page synchronization](web-api.md#page-synchronization)), and a live view's
module on its stream after a quarter of a second
([Delivery](../browser/live-view.md#delivery)). So a socket that brings
nothing for 75 seconds, two and a half of the longest heartbeat, is broken:
the page closes it and connects again. A live view shows a stall after a
second already, and a stall that ends within the 75 seconds, as when a Cloud
pauses for a checkpoint, keeps the view.

A socket that closes, is taken as broken, or cannot be made connects again
after a second, then after twice as long each time, up to 30 seconds, each
wait shortened by a random part so that the pages of all users do not return
at once after a restart. A channel that the backend closed because it shuts
down is the exception: the page shows the restart screen and connects again
every one to two seconds instead
([A page of another build](#a-page-of-another-build)). The waits start over at a second once the socket
works again: the channel when its snapshot arrives, a conversation when it
opens, a live view when its first `state` arrives. Meanwhile the page keeps
the channel's copy as it was, and a conversation shows that it is connecting.
A conversation socket that is lost before the session answered `open`, as
when the backend restarts while the conversation opens, follows the same
rule; a session that refuses to open, answering `open` with an `error` or a
`rejected`, shows the conversation as failed instead.

A page that comes back does not wait. Its timers stop while its computer
sleeps, so the 75-second watch counts only the time the page was awake, and a
wait that began before the sleep still has the rest of its time to run. So
when the page becomes visible again or comes back online, the module checks
every socket at once. A socket whose last message came 75 seconds ago or more
by the clock is broken, and one heard from since is watched for the rest of
those 75 seconds by the clock. Every closed socket then connects without
waiting for the rest of its wait, one the return found broken among them,
even a conversation's that was still opening. For example, a laptop
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
| Permission requests | Each summary's `permissionRequests` and `permissionsRevision` | The sidebar's needs-you mark follows the count; a page that shows the conversation reads its requests when the revision is newer than its own ([Conversation permissions](web-api.md#conversation-permissions), [Revisions counted in memory](web-api.md#revisions-counted-in-memory)) |
| Work panel tabs | Each summary's `panelRevision` | A page whose panel is open reads the tabs when the revision is higher than its own, and shows its own changes over them ([Work panel](#work-panel)) |
| Preferences | `preferences` | A change shows at once, and the part it changed stays as the user set it until its write is answered |
| The account, workspaces, devices, providers, subagent settings, the Cloud and each plugin's state | Their parts | The page shows what the backend holds |

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
follows any change. The user's browser does not say why an upgrade failed, so a
page whose channel does not open asks `GET /api/auth/me`, whose 401 ends the
session as any 401 does ([Authentication](#authentication)). Until the first
snapshot the page shows its loading state; a first connection that fails shows
the failure with a retry, which connects at once.

### A page of another build

A tab can outlive the build that served it, and a page never runs against a
backend of another build: the server's release decides the web app as it
decides every other part ([Upgrades](../delivery/upgrades.md)). For example,
a user has Demi open while its server is upgraded:

```text
the channel closes with 1001 backend_closing
        |
        v
the page shows "Demi Is Restarting" over the whole app and connects again
        |
        v  the channel's snapshot arrives
   +----+-------------------------------+
   another build than the page's        the page's own build
   -> the page loads itself again       -> the restart screen goes, and the
      from the backend                     page goes on where it was
```

- **The restart screen.** When the synchronization channel closes with
  1001 `backend_closing` ([Page synchronization](web-api.md#page-synchronization)),
  the backend shut down on purpose and will be back, so the page covers the
  whole app with the restart screen of `web-ui`: "Demi Is Restarting", "This
  page goes on when Demi is back." Nothing under it can be used, since
  nothing works without the backend. The backend does not know whether it
  stops for an upgrade or a restart, so the screen does not say which. While
  it shows, the channel connects again every one to two seconds, at a random
  point of that interval, so that the pages of all users do not return at
  once; the doubling waits of [Liveness and reconnection](#liveness-and-reconnection)
  are for a backend that went away unexpectedly, which a lost connection
  without that close means, and which shows no screen.
- **Another build.** Whenever a snapshot names another build than the page
  runs, whether after the restart screen or after a laptop slept through a
  restart, the page loads itself again at once, without asking. Nothing the
  user typed is lost: a draft is in IndexedDB and on the backend from the
  moment it is typed ([Drafts](#drafts)). A page that loaded itself for a
  build and still gets another one, as when a cache in front of the backend
  serves an old `index.html`, does not load again: it shows the restart
  screen's error state, "This Page Could Not Be Updated", with the reason,
  and keeps the build it tried in `sessionStorage` so that it tries once per
  build.
- **Where the build comes from.** Each `vite build` of `web` names its build
  with a new random id. The page carries the id, and the build writes it
  beside `index.html` as `build.json`, `{ "build": "<id>" }`. A rebuild of the
  same sources is another build: comparing ids needs no version scheme, and a
  needless reload costs a moment.
- **Who compares.** The backend that serves a web app reads its `build.json`
  as it starts, and refuses to start when it is missing or does not read
  ([Serving the web app build](web-api.md#serving-the-web-app-build)). Each
  snapshot carries it as `webBuild`, and the page compares it with its own.
- **Development.** Vite's development server serves sources, which name no
  build, and the development backend serves no web app, so its `webBuild` is
  null and nothing is compared. A page and a backend of different revisions
  there show their disagreement as any page shows a state or an answer it
  cannot read ([Calls and states](../architecture/plugin-pages.md#calls-and-states)).
  The restart screen shows there too, when the development backend stops.

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
The tabs open in one of the user's browsers share this storage, and a page
writes a conversation's record only for a change its own user made, so a tab
never replaces what another tab wrote for a conversation it left alone.
Preferences kept in the user's browser
hold presentation-only choices. Work-panel width and per-conversation open/closed state use the same account-scoped
local preferences. Refreshing restores whether the panel was open; a conversation
without a saved choice starts closed. The panel's tabs and selection are saved
with the conversation ([Work panel](#work-panel)); the pinned tabs' data and
the address drafts remain in memory for the page lifetime. Switching accounts uses that account's
saved choices. These stores do not replace backend ownership or authorization.

New conversations begin with a local UUID. The first send creates the backend
record using that ID and opens it while the page keeps showing the
conversation: it never turns to loading, so the composer the user sent from
stays. Submissions retain their message ID until admission is
confirmed, so retry can reconcile history and queue state before resending.
Editing and Fork use their backend operation contracts; see
[Message editing](../agent/message-editing.md) and
[Conversation Fork](../agent/conversation-fork.md). The page reads whether a
message can be edited from its block type alone, as the backend does
([Editable blocks](../agent/message-editing.md#editable-blocks)).

Product adapters connect shared file interfaces to device filesystem APIs,
the working-tree change routes, the raw file routes behind
[file previews](file-previews.md), the blob route behind a transcript's media,
and uploads; the live browser view's source
to the user stream route; and shared account interfaces to provider, pairing,
and Cloud APIs. The main composer and the edit composer add a file through the
same upload adapter ([Attachments](product.md#attachments)); no message
carries a file's bytes. The product reports the time zone and languages of the
user's browser to the user's preferences when they change.
The work panel keeps one file selection and one change selection per
conversation.
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
unconfirmed composer's text to local storage, which the user's browser writes
at once, and the next page of the conversation takes that text over the record.
A page that opens with such a record shows it instead of the backend's draft
and saves it, unless the backend's draft is that text already, as when the
save on closing arrived; it then only takes the backend's revision. The tabs
open in one of the user's browsers share the record, so the last edit written is
the one kept, as the last save is.

**A new conversation** has only its local UUID until the first send creates
its backend record. Its draft stays in IndexedDB, in the user's browser where
it was started: other tabs there list it when they load, and other devices do
not see it. The first send empties the draft, and every later
draft of the conversation lives in the backend.

An edit of a sent message ([Message editing](../agent/message-editing.md)) is
not the draft: it stays in the user's browser where it was opened.

## Development and checks

Start the backend executable, `demi-backend`, which listens on port 3271 by
default ([Development backend](../backend/backend.md#development-backend) gives
the whole launch, and `cargo xtask dev` starts one with a scripted Cloud, the development
models `.env` turns on and a seeded account in one command), then run the root package script `web:dev` for the product
at `http://127.0.0.1:18934`. Vite forwards `/api` HTTP and WebSocket requests
to the backend; set `DEMI_BACKEND_URL` when the backend runs at another
address. The script runs Vite under Node, so development needs a system Node,
the one part of the toolchain that does: under Bun, which runs the
repository's other scripts (`bunfig.toml`), Vite's proxy cannot forward a
WebSocket, since Bun's `node:http` client never reports the backend's
upgrade, and the conversation would never connect. Create the first account
through the setup API (`POST /api/setup`).
The script `web:build` writes `packages/web/dist`, which becomes the `web/` of
a server release, and the backend serves it from there
([Serving the web app build](web-api.md#serving-the-web-app-build)). The
script `web:gallery` runs the component catalog independently of credentials
and model services. Generated contract files are not committed; the scripts
that need them generate them first
([Generated TypeScript](../architecture/contracts.md#generated-typescript)).

Changes require the affected web app package typechecks, appropriate mocked
integration tests, and verification in product and gallery. Tests use
disposable accounts and captured mail, and never call real models. The
web app contract suite in `web` runs the product's API client and
`ConversationClient` against the backend executable
([Web app contract suite](../delivery/scenarios.md#web-app-contract-suite)).
Backend acceptance and deployment requirements belong to
[Delivery and acceptance](../delivery/roadmap.md).
