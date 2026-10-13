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
- **A choice the user must make is a wizard at the centre.** Steps the
  user answers before going on, such as choosing a password at the first
  sign-in and then the passkey offer, are a wizard: on a computer one
  dialog at the centre of the page, over the dimmed page, whose steps
  replace each other in place; on a phone the whole screen, as iOS's own
  setup does. A step that cannot be skipped has no close button; a toast
  or a card at an edge only informs and never asks.
- **Only a failure interrupts.** When the backend or the Host refuses what
  the user did, the page says so where it happened and offers to try again,
  and shows what is really there again.

This holds for every action whose result the page can tell, with one
mechanism. For example, the user archives "Fix the login test": the row
leaves the sidebar and the toast with Undo shows in the same frame, and the
request goes out behind them. Undo pressed before the backend answered
brings the row back at once and sends the restore after the archive. Had the
backend refused the archive, because the conversation had started running
in another tab, the row would come back where it was and a toast would say
why.

- **The page's state is what it last read, with its own changes on top.**
  Every piece of state the page reads from the backend is a *slot*: the
  value it last read, the revision that value carries, and the changes the
  page sent and has not seen land. The page shows the value with those
  changes applied. The product state's parts are slots, and so are the
  state of one conversation that is not its transcript: its messages saved
  for later, its draft, its attached Hosts and its work panel's tabs; so is
  a plugin's state, which a plugin page changes through the same call. A
  change leaves its slot when its answer is applied, or, when a newer value
  of the slot arrived before the answer, with that next value. A refused
  change leaves the slot, the page shows what the backend has, and the
  refusal is reported as any failed write is. One mechanism serves every
  slot, written once in `web-ui`; no action keeps its own copy of a value
  it is waiting for, its own rollback, or a flag that disables a control
  until an answer comes.
- **An answer releases only what it carries.** A write's answer says the
  revision of the slot it produced, and the change it answers leaves the
  slot when the page holds a read of that revision or a later one, never
  earlier: an answer that adds one item to a list is not the list, so it
  releases its own change and no other, and a read the page asked for
  before an answer landed, which may not include it, never removes a
  change that landed. Two pieces of state the backend changes in one
  transaction are one slot: a conversation's draft and its messages saved
  for later are, since saving the composer's message and editing a saved
  one change both at once.
- **A refused change never loses what the user wrote.** Dropping a refused
  change shows what the backend has, but text the user wrote is never
  only in a change: a message whose save was refused stays in the list as
  *Not Saved*, with Retry and Delete, and a composer message an edit was
  moving into the list stays there the same way, whatever the user did
  after. The user removes it; a refusal never does.
- **What the page creates, it names.** A record the page shows before the
  backend answers, such as a message saved for later or a forked
  conversation, gets its id from the page, a random UUID, so it stands in its
  list at once and the change that creates it can be sent again without
  making two. The backend takes the page's id and answers a second create
  with the same id and the same content as the first. A write that creates
  nothing, such as editing a saved message, carries an operation id the
  same way, and the backend keeps each such answer for a day, so a write
  sent again answers as it first did whatever happened since. A record that
  comes back is a new one: Undo after Delete names a new id, never the
  deleted one's.
- **Changes to one thing go in order; changes to different things do not
  wait for each other.** The changes of one thing, such as one
  conversation, are sent one at a time, in the order the user made them, so
  a later one never overtakes an earlier one: an Undo pressed right after
  Archive reaches the backend after the archive, never before it. Changes of
  different things go out side by side, so marking one conversation read
  never waits behind another's archive, which may wait for a paired device.
  A change of several things, such as archiving a selection, waits for the
  changes on its way of each of them. A control is never disabled while an
  earlier change of the same thing is on its way.
- **Which actions show at once.** Being able to tell the result is not
  enough; an action shows at once when it is frequent, light and easily
  taken back, as a switch in macOS's System Settings takes effect as it is
  flipped: archive and restore with Undo, pin and unpin, renaming,
  reordering, marking read, a conversation's model, effort and tier, moving
  it to a project, its messages saved for later (saving, editing, sending,
  deleting with Undo), switches, choices and fields that save themselves,
  the active account, enabling models.
- **No click goes unanswered.** Every press of a control changes the
  screen in the same frame: with its result, when it shows at once, or
  with the control showing that it is working, when it waits. A button
  that waits shows a spinner in place of its icon or beside its label and
  keeps its size; a menu item that waits closes its menu and the row it
  acts on shows the spinner; a row action shows it on that row. The
  control stays pressable for what makes sense while it waits, such as
  Cancel, and a second press of the same action does not send it twice.
  This is one behavior of the controls themselves, written once in
  `web-ui`: a control given an action that returns a promise shows that it
  works until the promise settles, so no handler can forget it.
- **Which wait, and show it in place.** An action waits for the backend's
  answer, showing its progress on the control that started it, when the
  user needs to know it really happened before going on:
  - **it ends access**, signing a browser out, signing an account out
    everywhere, disabling or enabling an account, changing a role, removing
    a passkey, revoking a device, detaching a Host from a conversation;
  - **it removes what other work depends on**, removing a provider entry, a
    credential, a subagent profile or a project;
  - **it is a form the user submits**: a dialog or sheet with Save or Add,
    which keeps its fields until the backend took them and shows a refusal
    beside them, such as adding or editing a credential, a subagent profile,
    a provider entry or its models, or saving the instructions;
  - **only the backend or another machine can tell its result**: signing in
    and changing the email or password; a password, a sign-in code or a
    pairing code the backend makes; a key a vendor must accept, such as an
    Exa API key or a provider's Test; creating a project, whose directory
    the Host must confirm; pairing a device; adding a skill source, which
    the backend fetches; an upload's progress; moving a conversation to
    another Host.
  Sending a message has its own states
  ([Recovering an unfinished turn](product.md#recovering-an-unfinished-turn)).

### Keyboard

Every control is reachable with Tab, in the order it reads, icon buttons
included: a message's copy and fork, a row's actions, a window's close. The
control that has the keyboard's focus shows the focus ring, and only a
keyboard focus does (`:focus-visible`), so a click leaves no ring; Enter and
Space act on it as a click does. The ring is shown whole wherever the control
stands: one ring for every focusable element, drawn outside the control, and
inside it within a box that clips, such as a scroller, a folding section or
a bubble, so no edge ever cuts it. The ring shows only where the user moved
the focus with the keyboard, by Tab or by the arrow keys in a list: never
after a click, a link's included, and never where the page moved the focus
itself, as when a closed window or menu gives the focus back to what opened
it, even when Escape closed it, nor on a field the page focuses on opening.
A selected item shows its selection by its own selected look, never by the
focus ring. A link in running text, a Markdown link, a file's or an agent's,
draws no ring: focused from the keyboard it takes its pointed-at look, its
underline and tint, as a link does in a browser's reading view, so a ring
never boxes a word in a sentence. A list that takes the keyboard focus rests
on its selected row, else its first, and rings that row. The browse tool's `demi.focusRings` finds any ring a page cuts
or covers ([Product checks](../delivery/browse.md)). A disabled control is skipped. One web-ui
primitive per kind of control owns this, so no caller makes a control
focusable by hand.

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
  page's lifetime. Its id is its kind's id. Pinned tabs are tabs of the
  strip's one tab list, before the others: they select, and the arrow keys,
  Home and End move across them, as across every tab.
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
  first tab, the Change view. A page selects a tab without its user's
  click only when the tab's kind says that something asked that the user see
  it, as the browser does for a tab the agent shows ([Showing a tab](../browser/live-view.md#showing-a-tab)); it is
  an ordinary selection and enters the history.
- **Contents stay.** A tab's content, once shown, stays on the page until
  the tab closes, the panel closes or the panel shows another conversation.
  Selecting another tab hides it with everything it shows: its scroll, its
  folded and unfolded rows, its picture. Selecting it again shows it as it
  was, with no entrance animation, since nothing in it changed. A hidden
  content is told it is not shown and stops what costs while nobody sees it,
  as the browser's live view does
  ([Live browser view](../browser/live-view.md#ending-a-view)).
- **Opening what is already shown keeps it.** A control in the transcript
  that opens something in the panel, such as a changed file's pill or a link
  to a file, opens the panel on it, and when the panel already shows exactly
  that, leaves it as it is; it never closes the panel. The panel closes by
  its own control.
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

## The phone layout

A window narrower than 768 pixels is a phone, and that width is the one
switch: everything below holds there and nowhere else, so a narrow desktop
window looks as a phone does, and a touch laptop or a tablet wider than that
is a desktop. One web-ui composable owns the switch, and CSS reads it as a
`phone:` variant. The desktop layout does not change.

For example, on an iPhone the user sees one bar over the conversation, the
sidebar button, the title and the panel button, and nothing else above the
transcript; a tap on the title opens a sheet of what can be done with the
conversation; the panel button covers the screen with the work panel, and
*‹ Conversation* comes back.

- **The page stays put.** It never zooms, drifts or bounces: the viewport
  forbids zoom, the page blocks pinches and overscroll, the app is the
  visible viewport's height, and only inner lists scroll, each keeping its
  overscroll to itself. A picture zooms by its own pinch, in the media viewer
  and the File view. Every text field is 16 pixels, so iOS never zooms on
  focus. The bars and the composer stand inside the safe areas, and with
  the keyboard open the composer sits on it and the transcript keeps its
  end in view: the user sees the messages above, the composer, then the
  keyboard's bar, and never types blind. Safari on iOS ignores the
  viewport's `interactive-widget` and keeps the page its full height under
  the keyboard, scrolling it to show the field, so the app follows the
  visual viewport itself: its height is the visual viewport's, it stands at
  the visual viewport's top, and the page's own scroll stays at 0. This one
  rule holds at every width, since it changes nothing where no keyboard
  covers the page: an iPhone turned sideways is wider than 768 pixels and
  shows the desktop layout, and its keyboard still never hides the composer.
  A sheet's or a window's bottom stands above Safari's floating toolbar as
  above the keyboard.
- **Sized for a finger.** Every row a finger picks, in the sidebar, sheets,
  Settings and the file tree, is at least 44 pixels tall, with 16-pixel
  titles; a group label is 36, the sidebar's footer 52, a tab 36. No tap
  highlight shows, a control's long press selects nothing and opens no
  callout, a message's long press still selects its text, and a long press
  opens the menu a right-click opens on the desktop. Controls the desktop
  shows on hover show at rest, a sidebar row's on the open row.
- **One bar.** The sidebar button; the conversation's title, centered, one
  line, with a chevron, and a status dot before it while the Host is not
  online; the panel button. A tap on the title opens a sheet with the
  sidebar row's own menu, without Open, and *Runs On*, the Host and the
  working directory. Rename asks in a dialog.
- **Whole-screen panels.** The work panel slides over the whole screen, its
  bar *‹ Conversation*, its tabs in a strip that scrolls sideways, and the
  strip's controls; the Agents and Running windows rise as whole-screen
  sheets. The sidebar is a drawer over a scrim, 85% of the width and at most
  320 pixels, so the conversation shows beside it; a tap on the scrim or a
  swipe left closes it. In the File view the tree covers the file's whole
  width and slides away when a file is picked.
- **Menus are sheets.** Every menu rises from the bottom as a sheet, one
  web-ui primitive: it closes on a tap outside it, a pull down past 30% of
  its height or a flick, Escape and Back, and springs back from a shorter
  pull; a pull down at the top of its scrolled content becomes the close;
  it stops at 90% of the screen and scrolls; focus moves in and returns to
  the control that opened it; the page behind does not scroll.
- **Back closes before it leaves.** The browser's Back closes the sheet,
  window, panel or drawer on top before it leaves the page, so a phone's
  back gesture undoes what the last tap opened.

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
loading does not wait for model discovery; starting a conversation's session
does, for the provider entries the page holds when it starts, so a change to
an entry while a conversation opens makes the session wait for the list of
the new entries rather than start with none. A message sent meanwhile shows
Requesting and goes once the list answers. A list that fails to load ends
that message as undelivered with the list's own failure, *Couldn't load the
models: …*, and Retry.

**Opening a conversation shows in two steps, each one change.** First, when
its history and its agents have both been read, the transcript and
everything that names its agents appear together: the messages, the agent
links in them, the dock's Agents chip, the Agents window and its Completed
menu; until then the pane says it loads and none of them shows. Then, when
the session opens, what only the live session knows joins them: the turn's
phase, running commands, the Running and Resume chips, the context use.
Until that second step, the conversation's status, in the sidebar's dot
included, stays what its summary says; the page never computes a status
from a phase it has not heard. A first opening that fails to connect is
still an opening, not a reconnection: it shows the first step if its
history was read, says that it connects, and never shows the second step's
parts from before. The agents list the history read returns is the live
tree's own when the conversation runs, the same list the session's
`subagents` frame then sends, so the second step never takes back an agent
the first showed ([Protocol](../agent/subagents.md#protocol)). A
conversation that opens no session, archived or with no model, stops at the
first step.

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

### Requests for one action

A page may be a long way from its server: a laptop behind a proxy, reaching a
server on another continent through a TLS edge, waits about half a second for
each round trip. So one thing the user does costs as few round trips as the
work allows, and never one per item:

- **Requests that do not depend on each other go out together.** Opening a
  conversation reads the latest page of its transcript, its subagents, its
  attached hosts, its draft, its permission requests and its panel at once,
  and connects its socket beside them; the transcript does not wait for the
  hosts. The socket's `open` names the blocks the page read
  ([Where a reset starts](../agent/runtime.md#where-a-reset-starts)), so it
  goes once both the page and the connection are there; the socket connects
  meanwhile. The first load starts the
  synchronization channel beside the session check, and opens the
  conversation its URL names by its id, without waiting for the channel's
  first state, unless the URL is a new conversation the browser made and has
  not sent yet, which has nothing to read: the page knows it by the history
  entry's mark and opens its saved draft instead.
- **An action that changes several things is one request.** Creating a
  conversation carries its settings and its hosts in its own request
  ([Conversation creation and Fork](web-api.md#conversation-creation-and-fork)),
  and the page shows the conversation it created without reading it back;
  Close Others is one change of the panel
  ([Work panel state](web-api.md#work-panel-state)).
- **An answer that says what happened is not followed by a read.** A change
  of the panel answers the revision it made; the page reads the panel only
  when the revision shows that another change came in between. A decision on
  a permission request, an attached host's change and a provider's settings
  change likewise update the page from their answer and the synchronization
  channel, not from a second request.
- **Several uploads run at once,** up to four at a time: dropping a folder
  uploads its files, whose writes make their folders, and makes only its
  empty folders; no listing follows each file: the tree adds each entry it
  uploaded, with the name and size it sent, when its upload succeeds, and the
  Host's watch then reports the folder, which is listed once for the reports
  of 100 ms ([What the service keeps](../architecture/plugin-pages.md#what-the-service-keeps)).

### Liveness and reconnection

A page holds WebSockets of four kinds to the backend: the synchronization
channel, a socket for each open conversation, the stream of each
[live browser view](../browser/live-view.md), and the
[file watch](web-api.md#file-watch) of the conversation whose files it shows,
and the signaling socket of each device it has a
[direct channel](../execution/direct-channel.md) to, whose long-lived
operations keep the same heartbeats over the channel. All of them tell a quiet socket
from a dead one by one rule and connect again by another, and one module of
`web-ui` implements both. For example, a laptop sleeps and its network drops
without a close. Nothing tells the page: its sockets still look open, a
conversation would go on showing a turn as running, and a live view would go
on showing its stream as stalled. The far end of each socket sends a
heartbeat once it has sent nothing else for a while: the backend on the
channel, on each conversation socket, on the file watch and on a direct
channel's signaling socket after 30 seconds
([Order and delivery](../agent/runtime.md#order-and-delivery),
[Page synchronization](web-api.md#page-synchronization)), and a live view's
module on its stream after a quarter of a second
([Delivery](../browser/live-view.md#delivery)). So a socket that brings
nothing for 75 seconds, two and a half of the longest heartbeat, is broken:
the page closes it and connects again. A live view shows a stall after a
second already, and a stall that ends within the 75 seconds, as when a Cloud
pauses for a checkpoint, keeps the view.

A socket whose opening handshake has not completed within 10 seconds cannot
be made: the page abandons it, as it does one refused, since a browser waits
minutes before it gives up on a handshake that a proxy in front of a
restarting backend holds. A socket that closes, is taken as broken, or cannot
be made connects again after a second, then after twice as long each time, up to 30 seconds, each
wait shortened by a random part so that the pages of all users do not return
at once after a restart. A channel that the backend closed because it shuts
down is the exception: the page connects again every one to two seconds
instead
([A page of another build](#a-page-of-another-build)). When the channel's
snapshot arrives after the backend was away, every other socket of the page
that waits to connect connects at once, as at the page's return, so no part
of the page still says it is connecting after the banner went. The waits start over at a second once the socket
works again: the channel when its snapshot arrives, a conversation when it
opens, a live view when its first `state` arrives. Meanwhile the page keeps
the channel's copy as it was and shows the connection banner
([A page of another build](#a-page-of-another-build)), and a conversation
shows that it is connecting.
A conversation socket that is lost before the session answered `open`, as
when the backend restarts while the conversation opens, follows the same
rule; a session that refuses to open, answering `open` with an `error` or a
`rejected`, shows the conversation as failed instead.

#### A request the drop interrupts

A request the page sent on a conversation socket that drops before the
session answers it is neither refused nor sent again. For example, the user
presses Stop on her phone as it loses its network; the page cannot tell
whether the session stopped the turn, so it shows a toast, *Connection Lost*
with *Demi may not have received your request. Check the conversation once
it reconnects.*, and the reopened socket's snapshot shows whether the turn
stopped. Sending it again could do it twice, such as a Retry that runs the
turn twice. A request the session had taken before the drop, such as a
Resume that was running, says nothing: the snapshot shows how it went. And
a request made as the page returns waits for the return's check, so it goes
on a socket the check found alive or on the new one, never on one the check
is about to replace.

#### What a reopened socket brings back

A socket that opens again brings the state it feeds as it is now, whole,
and the page replaces its copy with it: nothing of the socket's state from
before the drop stays, and the page asks nothing about what it held. Only
what is too large to send each time, a transcript, comes by its version: the
snapshot names each one's revision, and the page reads again the ones it
shows that moved. For example, a laptop sleeps while a subagent works; the
subagent finishes during the sleep; when the lid opens, the conversation
reopens, its snapshot lists the subagent as finished, and the page shows it
so, reading its last page because its window is open.

| Socket | The snapshot | Read by version |
| --- | --- | --- |
| The synchronization channel | The product state | The conversation-scoped parts whose revisions the summaries carry ([Page synchronization](web-api.md#page-synchronization)) |
| A conversation | The root's state, every subagent's summary, every running command, every node's context use ([Connections and the live tree](../agent/runtime.md#connections-and-the-live-tree)) | The root's transcript from the blocks the page holds, `from` and `edge`; a subagent's transcript, by the revision its summary carries, when the page shows it |
| The file watch | Each subscription live again | A shown file, by its version: the Host answers unchanged or the new contents ([File watch](web-api.md#file-watch)) |
| A live view | Its `state`, tabs and the watched tab's controls and downloads | The picture: a new key frame |
| A device's signaling socket | Nothing: the backend closes the device's end of the direct channel when the socket drops, so the page leaves the direct path at the drop and uses the relay until a new channel is made | Nothing |

Until the snapshot arrives, the conversation shows that it is connecting and
what it shows is the page's copy; the snapshot replaces it in one change.

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

The return reads what came in while the page was away before it judges a
socket. A browser that froze a hidden tab holds the messages that arrived
meanwhile and delivers them only once the tab runs again, so a heartbeat may
be waiting to be read when the page becomes visible: the check runs after the
page has read what its sockets hold, and a socket that brought anything is
not broken. Likewise, the browser's word that it went offline counts only
once it has held for a second: a network the system re-evaluates, as macOS
does when it changes its primary interface, may report offline for a moment
while every socket stays open, and the page neither shows the connection
banner nor closes a socket for that moment. An offline that holds shows the
banner and still closes no socket, since a socket that kept its connection,
as through a VPN's tunnel, goes on working: a socket ends by its own close or
its silence, and when the browser is online again the page checks each one
as at a return.

Each socket's end is recorded on both sides, so a drop the user saw can be
traced afterwards. The backend logs, at `info`, each page socket it opens
and each it ends: the socket's kind and conversation, an id that ties the
two lines, which is the [request ID](../backend/backend.md#request-ids-and-the-server-log)
of its upgrade, the edge's request id and the client's address when a proxy such
as Cloudflare passes them, and, at the end, why it ended (the page closed it
with a code and reason, the connection broke without a close, the page fell
a whole outbox behind, the backend shut down), how long it lived and how
long since it last sent and last received. A socket that fell behind, an
internal error and a close the page did not complete in time are warnings.
The page reports through its error reports why it replaced a socket: its
silence, a close code, the browser going offline, or its return.

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
| Work panel tabs | Each summary's `panelRevision` | A page that shows the conversation reads the tabs when the revision is higher than its own, with its panel open or closed, since a tab a kind asks to show opens the panel, and shows its own changes over them ([Work panel](#work-panel)) |
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
snapshot the page shows its loading state. A page loaded while the backend
cannot be reached, as during a restart, has nothing to show yet: it says
"Connecting to Demi…" where the app will be and waits, connecting again as
any channel does, and never sends the user to sign in for it; only the
backend's own answer, a 401, does. A first connection the backend answers
with its own failure shows that failure with a retry, which connects at
once.

### A page of another build

A tab can outlive the build that served it, and a page never runs against a
backend of another build: the server's release decides the web app as it
decides every other part ([Upgrades](../delivery/upgrades.md)). For example,
a user has Demi open while its server is upgraded:

```text
the channel closes with 1001 backend_closing
        |
        v
the page shows the banner "Demi is restarting" and connects again
        |
        v  the channel's snapshot arrives
   +----+-------------------------------+
   another build than the page's        the page's own build
   -> the page loads itself again       -> the banner goes, and the page
      from the backend                     goes on where it was
```

- **The connection banner.** When the backend cannot be reached, the page
  says so in a banner across the top of the app, as Slack and Linear do, and
  everything under it stays readable: the conversation, its panel, the
  sidebar. The banner says what is happening in the user's words: "You're
  offline" while the browser reports no network, "Demi is restarting" after
  the channel closed with 1001 `backend_closing` (the backend shut down on
  purpose and will be back; it does not know whether for an upgrade or a
  restart), and "Reconnecting to Demi" once a lost channel's first new
  attempt failed. It goes when the channel's snapshot arrives. The banner is
the one place that says so: while it shows, a conversation whose socket waits
for the same backend shows no Connecting row of its own, which it shows only
when its socket is lost while the page still reaches the backend. A message sent
  meanwhile shows in the transcript as waiting to be sent and goes when the
  conversation opens again, with its id, so it is sent once; it never shows
  as failed for the lost connection. Every request of the page waits so: the
page's one HTTP client holds a request that does not reach the backend, a
read or a write alike, and sends it once the page reaches the backend again,
so no control and no region shows a failure about the connection. A control that needs the backend, such
  as a setting or a Retry, stays as it is and does what it does once the page
  is back, never a failure about the connection. While the backend shuts
  down on purpose, the channel connects again every one to two seconds, at a
  random point of that interval, so that the pages of all users do not return
  at once; the doubling waits of [Liveness and reconnection](#liveness-and-reconnection)
  are for a backend that went away unexpectedly.
- **Another build.** Whenever a snapshot names another build than the page
  runs, whether after the banner or after a laptop slept through a
  restart, the page loads itself again at once, without asking. Nothing the
  user typed is lost: a draft is in IndexedDB and on the backend from the
  moment it is typed ([Drafts](#drafts)). A page that loaded itself for a
  build and still gets another one, as when a cache in front of the backend
  serves an old `index.html`, does not load again: it shows the error
  "This Page Could Not Be Updated" over the app, with the reason,
  and keeps the build it tried in `sessionStorage` so that it tries once per
  build. The page reads a snapshot's `webBuild` before the rest of it: a
  snapshot of another build whose other parts this page cannot read, as when
  the new release added a value the old page does not know, still loads the
  new build, and never leaves the banner up for good.
- **What the reload opens.** The page loads itself at the address it shows,
  and a page that showed a new conversation, which has no record before its
  first send, opens a new conversation again, never "Conversation not
  found".
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
  The connection banner shows there too, when the development backend stops.

## Transcript windows

A page holds the parts of a transcript the reader has been to, not the whole
of it. For example, the user opens a conversation of 3,000 blocks: the page
reads the latest page, blocks 2,940 to 2,999, and shows it at its end
([Pages](web-api.md#pages)). The user scrolls up; near the top the page reads
the page before, and its rows appear above without moving the rows the user
reads. Later the user opens a search result in block 812: the page reads the
page around it, blocks 790 to 860, and shows it with the block in the middle,
marked for a moment. It now holds two windows:

```text
held:   [790 ──── 860]                    [2,880 ─────── 2,999]
         shown: around the search result    latest: at the end, live
```

- **Windows.** For each agent the page holds windows: runs of consecutive
  blocks, each knowing whether it reaches the transcript's start and its
  end. Windows that meet join into one, as blocks' indices say exactly where
  each ends. The latest window, the one that reaches the end, is always held:
  the stream's reset and patches apply to it as they come, and to any other
  window that holds the block they name
  ([Where a reset starts](../agent/runtime.md#where-a-reset-starts)). A
  window stays held while the conversation stays in the page's cache, and
  switching back to the conversation shows it as the reader left it.
- **One window shown.** The transcript shows the window the reader is in:
  the latest at its end on a first opening, the window and the place the
  reader left on a return, the window around a block a search result opens
  or a report's title goes to. A subagent's panel shows one window, from
  its latest back, as it has no scroll-to-bottom button: a block before it,
  such as the call a report in the panel names, is reached by reading the
  pages before the window until one holds it.
- **Reading on.** When the reader comes within a screen of an edge of the
  shown window that is not the transcript's start or end, the page reads the
  next page there, and the edge shows a loading row until it arrives. Rows
  added above keep the rows in view where they are, as a chat app keeps its
  place when earlier messages load. A page that reaches another window joins
  it, and the reader scrolls on into it.
- **The latest.** While the shown window is not the latest, the
  scroll-to-bottom button shows, as it does whenever the end is out of view:
  it shows the latest window at its end. Meanwhile new activity goes into the
  latest window and the shown one stays still; sending a message shows the
  latest, as it always scrolls to the message sent.
- **Opening a row.** A folded row whose block the page read in its light
  form ([Light form](web-api.md#light-form)) reads the block whole when the
  user opens it and shows it in place, with the row's loading state until it
  arrives; the page keeps it whole after.

What the page derives from the transcript stays within a request, which a
page never cuts: work groups, a request's changed files and its line under
the request, how long a thinking took, the message the page offers to edit
and the answer that offers Regenerate. A Change tab that names a request
the page does not hold reads the page around its `user` block. What a request
does not hold, the page reads with the windows: the context card's
instructions, from the latest window's newest instructions block or else
the page's `instructions`; a compaction divider's size, from its boundary
or else the page's `summaries`; the call that started a command a report
names, from the command's record; the subagents, from their list and the
`subagent` frames, each subagent's transcript in windows
of its own when the panel shows it
([Conversation history](web-api.md#conversation-history)).

### Rewrites

A retry, an accepted edit or a compaction rewrites the transcript, and
indices then name other blocks. The stream tells the page as patches: a
`truncate` drops every held block from its index on, in every window, and an
`add` before a window's end moves the indices after it
([Patches and versions](../agent/runtime.md#patches-and-versions)). A rewrite
can also land between a read and the stream, or between two reads. A page
read whose `edge` no longer matches answers `transcript_changed`, and a reset
whose `from` no longer matches starts at the latest page
([Where a reset starts](../agent/runtime.md#where-a-reset-starts)). Either
way the page drops the windows it can no longer join, keeps the latest the
stream gave it, and reads the page around the block in view again, so the
reader stays where they were; when the rewrite removed that block, the page
shows the latest at its end, as after an edit it shows the new answer.

## Authentication

The page holds one session state: checking, setup needed, signed out, or
signed in with an account. Startup checks `GET /api/auth/me` before the
initial route mounts, and the account in its answer is validated before it
enters the state. Without a session, startup asks `GET /api/setup` whether
the instance still needs its master account. Later route changes use the
current state, so opening a local conversation does not wait for an
authentication request. Each state has its pages: setup needed only
`/setup`, signed out only `/login`, and signed in the product, which sends
`/login` and `/setup` to the chat. A page opened while signed out, such as a
link to a conversation, is kept as `/login?next=<path>`, a path inside the
app only, and signing in goes on to it, as GitHub and Slack do. Settings have addresses of their own, `/settings/<section>`, over
the page they opened from: a link opens a section, Back closes the dialog or
returns to the previous section, and a reload keeps it open; `/settings`
alone is the list of sections on a phone and the first section elsewhere.
A paired device has a page of its own under Devices,
`/settings/devices/<id>`, which its row in the list opens, as a row of
macOS's System Settings opens its detail
([What the user sees](../execution/direct-channel.md#what-the-user-sees)).

On an instance without accounts, the first visitor creates the master account
on the setup page: a name, which becomes the account's nickname, the email,
and a password with its confirmation, each as the
[Account API](web-api.md#account-api) bounds it.
`POST /api/setup` creates it and signs it in, and the page opens the chat.
Another visitor who set the instance up first makes the request answer 404
`already_set_up`; the page then says the instance is set up and offers only
signing in. Until the master account exists, anyone who reaches the instance
can create it, so an operator creates it right after installing
([Installation](../delivery/installation.md)). A `GET /api/setup` that fails
leaves the visitor on the sign-in page, whose own request then reports the
failure.

The session is the backend's HttpOnly cookie
([Authentication and ownership](../backend/backend.md#authentication-and-ownership)),
which accompanies every same-origin request; JavaScript never receives a
session token. Passwords stay in the form and its request and are cleared
after login.

The sign-in and setup forms are `web-ui` components on one shared page
layout, and each owns its busy and error states; the setup form also checks
the password's length and its confirmation as they are typed.
`web` supplies their request handlers; the gallery shows the same components
in fixture phases and never calls the backend. `POST /api/auth/login` sends the
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
progress; and each conversation's scroll position. Its records follow the
same rule as the backend's stores between published releases
([Schemas and migrations](../backend/storage.md#schemas-and-migrations)): a
change of a record's shape raises the database's version, and its upgrade
turns the previous release's records into the new shape, as a message on its
way, one per conversation in 0.1.21, becomes a list of one. A record that
still does not read is reported once, in words and without its raw contents,
and set aside; every other record is restored, so one bad record never costs
the user their other drafts. A conversation with a
record keeps its model settings only there
([Sidebar mutations and read state](web-api.md#sidebar-mutations-and-read-state)).
The tabs open in one of the user's browsers share this storage, and a page
writes a conversation's record only for a change its own user made, so a tab
never replaces what another tab wrote for a conversation it left alone.
Preferences kept in the user's browser
hold presentation-only choices. Work-panel width, the dock windows' height as a share of the session's, and per-conversation open/closed state use the same account-scoped
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
  shows in the transcript under the Requesting row
  ([Recovering an unfinished turn](product.md#recovering-an-unfinished-turn)), but the saved draft
  stays until the
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
the whole launch, and `bun xtask dev` starts one with a scripted Cloud, the development
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
