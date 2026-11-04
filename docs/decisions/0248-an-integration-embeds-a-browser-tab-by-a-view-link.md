---
status: proposed
---

# An integration embeds a browser tab by a view link

## Problem

An agent of an integration opens `https://accounts.example.com/login` in its
conversation's browser, tab `t3`, and the site asks for a password the agent
must not know. The person the agent works for is in the integration's own
product, such as a chat app, and has no Demi account, so the work panel's
live view, which a session reaches, is not theirs to open. Today the program
has no way to show them the tab: it can only turn the browser capability off
and run a remote desktop of its own in the Cloud, beside Demi's browser and
apart from it.

## Decision

The program asks for a link to one tab of a conversation's browser, and the
person opens it, or the product shows it in an iframe:

```text
POST /v1/conversations/{conversation}/browser/tabs/t3/views
{ "expiresIn": 900, "readOnly": false }

201 { "id": "4f…", "tab": "t3", "readOnly": false,
      "expiresAt": "2026-03-02T10:15:00Z",
      "url": "https://demi.example.com/v/Yx2…" }
```

The link's page is the tab's picture and nothing else:

```text
┌──────────────────────────────────────────────┐
│                                              │
│      accounts.example.com's page, live       │
│      (clicks and keys reach it, or none      │
│       when the view is read-only)            │
│                                              │
└──────────────────────────────────────────────┘
```

### The link

* `POST …/browser/tabs/{tab}/views {expiresIn, readOnly}` makes one. Both
  fields are required: `expiresIn` in seconds, any positive number, and
  `readOnly`. The integration's browser capability must be on (0149), and
  the tab is named by the id the agent's `demi browser` commands print,
  which no other tab of the conversation ever takes.
* `GET /v1/conversations/{conversation}/browser/views` lists the views that
  have not expired, as the answer above, without `url`, so a program that
  reconnects reads where it stands (0221).
* `DELETE /v1/conversations/{conversation}/browser/views/{id}` removes one.
  Removing it, or its expiry, ends each page open on it. Turning the
  integration's browser capability off, or deleting the integration, ends
  each page open on its views as a link that opens nothing; a view that has
  not expired opens again once the capability is back on.
* The link holds a random token. Demi keeps its SHA-256, not the token, as
  it keeps a lending key's, in `browser_views` (`id`, `token_hash`,
  `conversation_id`, `tab`, `read_only`, `created_at`, `expires_at`), which
  the conversation's deletion takes with it. An expired view is deleted
  with the next view made in the conversation.

### The page

* `/v/{token}` is a page of its own build entry, as a share's `/s/{token}`
  is: no session, nothing of the account, and every miss, an unknown token
  or an expired or removed view, the same "This link has expired or was
  removed". It reads `GET /api/views/{token}`, the tab and whether the view
  is read-only, or `404 view_not_found`. Its Content Security Policy is the
  share page's, its connections only its link's routes, and
  `frame-ancestors *` where every other page of Demi has `'none'`: the token
  is the key, and a product embeds the page wherever it shows it.
* It draws the tab with the work panel's live view (`LiveView.vue`): the
  picture, the cursor, the page's native controls such as a select's list,
  its JavaScript dialogs, and the right-click menu's Copy, Cut and Paste. It
  has no address bar, tab list, downloads or viewport menu, and the menu
  has no Back, Forward or Reload, which the work panel asks of the browser
  through calls of the user's session.
* A read-only view takes no input: no pointer, key, menu or dialog of its
  own.
* While the Cloud is stopped or the browser does not run, the page says so,
  in the work panel's words, and shows the tab once it runs; a stream that
  breaks opens again as the work panel's does.
* The page tells its parent window how it stands, with `postMessage` to any
  origin: `{ type: "demi.view", state: "live" }` once the picture shows, and
  `{ type: "demi.view", state: "ended", reason }` once it ends for good,
  `reason` being `expired`, `removed`, `not_found`, a link that opened
  nothing, or `tab_closed`. The product closes its frame on `ended`.

### The backend

`/api/views/{token}/stream` is the page's WebSocket. It finds the view, then
opens the browser's user stream (`browser.live`) on the conversation's Host
as the work panel's lane opens it (`open_user_stream`), with the view's
scope as the invocation's arguments, and relays its bytes until the view
expires, is removed, or either side ends. The relay is the one user streams
have (`crates/backend-http/src/streams.rs`), over this socket instead of a
lane of the page connection; it carries the bytes and decides nothing else.
The socket closes with `view_expired` or `view_removed`, with
`view_not_found` when the view was gone as it opened or no longer opens, and
with the code of the Host's refusal, such as a stopped Cloud's, as a lane's
refusal says it.
The pages open on each view are kept in memory, so a removal ends them, and
so does the integration's change or deletion that the shard makes.

### The browser's module

What the view may do is the module's to enforce, since the backend relays
bytes it does not read. The stream's invocation takes arguments, which the
work panel's leaves out:

```rust
pub struct LiveInput {
    /// The one tab the viewer sees; none for every tab.
    pub tab: Option<TabId>,
    /// The viewer only watches.
    pub read_only: bool,
}
```

* A viewer of one tab watches that tab once the browser lists it, hears of
  it alone in each `State`, and its `Watch` of any other is dropped. Once
  the tab closes, its `State` of a running browser lists none, and the page
  ends with `tab_closed`. Its `Mode` is dropped, as it has no viewport
  menu, it hears no `Downloads`, as it has no downloads list and a
  download's path is the Host's, and its panel decides only its tab's size,
  never the browser's screen or the size of a tab the user opens, which the
  work panel's viewer decides (`hub.rs`, the driver).
* A read-only viewer's `Input`, `Paste`, `Choice`, `Upload` and its file
  frames, `Dialog` and `Recapture` are dropped, and its panel never decides
  a tab's size (`hub.rs`, `decider`): its picture is the size the agent's or
  another viewer's panel gives the tab, fitted into its frame.

## Spike

None: each part is one Demi has, the share's page and policy, the user
stream and its relay, and the live view's page and module.

## Prior art

* Browserbase gives each tab a live view URL to put in an iframe, and the
  frame posts `browserbase-disconnected` to its parent when the session ends
  ([Session Live View](https://docs.browserbase.com/platform/browser/observability/session-live-view)).
  Its read-only view is the embedding page's `pointer-events: none`, which
  the viewer can undo.
* Steel's live view takes `interactive=false` in its URL, and its URL needs
  no authentication
  ([View and Embed Live Sessions](https://docs.steel.dev/overview/guides/view-and-embed-live-sessions)).
* Demi departs from both: the link expires and can be removed, holds a
  secret Demi keeps only the hash of, and its read-only view is the
  browser's own refusal, so neither the URL nor the frame can undo it.

## Alternatives

* **The whole work panel's browser, every tab.** The person would see the
  agent's other tabs; one tab is what a takeover needs, and a program that
  needs two makes two links.
* **A link that names its tab and mode in its URL.** Anyone who has the link
  could change them.
* **A view the product draws itself from the stream.** Each product would
  build the live view's decoder, input and native controls again.

## Open questions

None.
