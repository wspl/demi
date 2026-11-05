---
status: proposed
---

# An integration shows a conversation by a view link

Supersedes: 0248 (An integration embeds a browser tab by a view link), the
parts that say a view link names a tab and that its expiry is required.

## Problem

A team chat app built on Demi starts a conversation for each thread. A
member asks it to rename a cookie, and the agent spends ten minutes on it,
with two subagents and a test run. The member sees the agent's last reply in
the thread and nothing of the work: which commands ran, what a subagent
found, how much context and money the conversation used. The app can read
all of it through the API and draw a transcript of its own, but that means
building again what Demi's page already shows, its blocks, windows and
context card, for every app.

## Decision

The program asks for a link to a conversation, and the person opens it, or
the app shows it in an iframe:

```text
POST /v1/conversations/{conversation}/views
{ "expiresIn": null }

201 { "id": "8c…", "createdAt": "2026-03-02T10:00:00Z",
      "expiresAt": null, "url": "https://demi.example.com/v/Qm7…" }
```

The page is what the conversation's page shows, for someone who only
watches:

```text
┌──────────────────────────────────────────────────────────┐
│                       ( Rename the session cookie … )    │
│  Thought for 2s ›                                        │
│  [Find the cookie assertion] looks for …                 │
│  Run the auth tests ›                                    │
│                                                          │
│  (3 agents) (5 running) (1 service) (5 scheduled)        │
├──────────────────────────────────────────────────────────┤
│ ● Running · 2m 35s   shop          Claude Opus 5.5 · High ◔│
└──────────────────────────────────────────────────────────┘
```

### The page

* The root agent's transcript, live, as the conversation's page shows it:
  thinking, calls and commands open, a reply streams, an agent's link opens
  the Agents window on its tab.
* The dock's chips, Agents, Running, Services and Scheduled, with no
  composer under them. Each opens its window or menu as on the
  conversation's page: a child's transcript and bar, a command's output, the
  schedules and when each wakes.
* Along its bottom edge, the bar the Agents window shows along a child's,
  for the root agent: its state and how long the turn has run, its folder,
  its profile, its model and effort, and the context indicator. The context
  card holds the context and the conversation's cost.
* Nothing acts on the conversation. There is no Stop Agent, Stop All, End
  Command, Compact, Fork or edit, and a schedule's switch is disabled, with
  no Delete. Copy Agent ID and Close Tab stay. A file, a commit or a branch
  is its text, since the page reaches no Host. The context card shows no
  usage limits and no instructions, which are the account's.
* It is built of the conversation page's own parts, each told that the page
  only watches (`readOnly`): the transcript list, `SessionSurface` and
  `SessionDock`, `SubagentPanel`, `TerminalPanel`, `SchedulesChip`,
  `AgentStatusBar` and the context indicator.
* It follows the reader's system for light or dark and for its language, as
  a share's page and a tab's view link page do.
* It tells its parent window how it stands, as a tab's view link page does
  (0248): `{ type: "demi.view", state: "live" }` once the transcript shows,
  and `{ type: "demi.view", state: "ended", reason }` once it ends for good,
  `reason` being `expired`, `removed` or `not_found`.

### The link

A view link names a tab of the conversation's browser, as 0248 made it, or
the conversation itself. Both kinds are the same record and the same page,
`/v/{token}`:

* `POST /v1/conversations/{conversation}/views {expiresIn}` makes a link to
  the conversation. It needs no capability of the integration's: the
  conversation is the integration's own.
* `expiresIn` is required, in seconds, or `null` for a link that never
  expires, as a share's does not; it opens until it is removed. A tab's
  link takes `null` too.
* `GET /v1/conversations/{conversation}/views` lists the conversation's
  links that have not expired, the newest first, without `url`.
  `DELETE /v1/conversations/{conversation}/views/{id}` removes one, which
  ends each page open on it.
* The conversation's deletion takes its links, and each page open on one
  ends with `removed`. The integration's deletion takes them too; its pages
  close with `view_not_found`, as 0248's tab pages do, and a conversation's
  page, finding its link gone, ends with `removed`.
* `GET /api/views/{token}` answers what the link shows, as a tagged union:

  ```rust
  #[serde(tag = "kind", rename_all = "camelCase")]
  pub enum ViewLink {
      Tab { tab: String, read_only: bool, expires_at: Option<Timestamp> },
      Conversation { expires_at: Option<Timestamp> },
  }
  ```

The table `browser_views` becomes `view_links`: `tab` is null for a link to
the conversation, `read_only` is true for it, and `expires_at` is null for a
link that never expires. A link to the conversation is read-only by what it
is, so its request has no `readOnly`.

### The backend

The page reads and follows the conversation as the conversation's page
does, with the token for its key in place of a session:

* The routes it reads, the conversation's record, the root's and a child's
  transcript pages, a block whole, the recent cost, blobs and the plugins'
  reply blocks, answer under `/api/views/{token}/…` as under
  `/api/conversations/{conversation}/…`. They are the same handlers, given
  the conversation the view names, as the share's routes are given the
  share's (`shares.rs`). A route the page does not read has no view twin:
  the agents and the commands' output come over the lane, and the schedules
  with the metadata.
* A blob answers when the conversation's transcript or its attachments show
  it, and a plugin's reply block when a text of the conversation names it,
  as a share's do; the owner's other blobs and blocks are not found.
* The conversation's record answers as
  `ViewedConversation { conversation, runsOn }`: the record the
  conversation's page reads, and the model the root runs on, named. The
  conversation's page names the model from the account's catalog, which a
  view does not read.
* `WS /api/views/{token}/connect` is a page connection that opens one lane,
  the conversation's. The open of another kind of lane is refused, 403
  `view_only`, and of another conversation's, 404 `view_not_found`, as for a
  conversation that is not there. The lane takes `open`, `follow`,
  `unfollow` and `close`;
  every other frame is answered `error` `view_only` and does nothing.
  `open` takes no draft, and `follow` takes the agents, the transcripts
  and, of the conversation's own values, the metadata alone; the saved
  messages, the workflows and the shares are the account's.
* Every miss answers the same 404 `view_not_found`, the connection's
  upgrade included, as an ended session's upgrade answers 401. An open
  connection closes with `view_expired`, `view_removed` or
  `view_not_found`, as a tab's socket does; the page then connects again,
  is refused, and reads the link to say which. The pages open on a view are
  kept in memory (`open_views.rs`), so a removal ends them.
* The view page's content security policy, 0248's, also lets the page
  reach its own `/api/views/{token}/…` routes and the plugins' reply blocks
  under `/api/blocks/`.

## Spike

None: the page is the conversation page's parts, the routes are its
handlers, and the link, the page entry and its policy are 0248's.

## Prior art

* LangSmith shares a trace by a public, read-only link that needs no
  account and opens until it is unshared, by its page or by the API's
  `unshare` call
  ([Share or unshare a trace publicly](https://docs.langchain.com/langsmith/share-trace)).
* Browserbase and Steel give a live view URL to frame in an iframe (0248).
* Demi departs from LangSmith in that its link is live, not a record of a
  finished trace, and in that the program chooses whether it expires.

## Alternatives

* **A share.** A share is the conversation as it stood when it was shared,
  read by a page that shows the messages and counts the work between them.
  The person here needs the work itself, live.
* **A page of its own build.** It would draw the transcript, the windows and
  the bar again, and drift from the conversation's page with each change of
  it.
* **A session for the viewer.** Signing the viewer in, with a cookie the
  frame may not keep across sites, would make an account of someone who has
  none.

## Open questions

None.
