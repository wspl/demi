---
status: accepted
---

# A subscription's model searches with its vendor's own search

## Problem

Alex asks a Grok Build agent what developers say on X about a release. Each
of Demi's three subscriptions has a search of its own: Codex has OpenAI's
web search, Claude Code has Anthropic's, and Grok Build has xAI's web search
and X search. Demi turns all of them off. The agent can only call
`demi web search`, which cannot see X, and which needs an Exa account or
shares an anonymous limit, though Alex already pays for a search with each
subscription.

## Decision

Each subscription entry gets a switch for its vendor's search, on from the
start, which the user can turn off. With it on, the entry's models search on
their own, and every search is an opaque block
([0175](0175-a-vendors-own-step-is-an-opaque-block.md)) with the `search`
view, a step of the run.

### Settings

Settings › Providers › Codex, Claude Code and Grok Build each get a
**Built-in Tools** section above the models, one card like the media
sections, with one switch per tool the vendor runs on its side:

```text
Built-in Tools
+------------------------------------------------------------+
| Web Search                                            [on] |
| The model searches the web with its vendor's own search.    |
|------------------------------------------------------------|
| X Search                                              [on] |
| The model searches posts and people on X.                   |
|------------------------------------------------------------|
| Image Understanding                                   [on] |
| The model looks at the images in the pages and posts it     |
| finds.                                                      |
|------------------------------------------------------------|
| Video Understanding                                   [on] |
| The model watches the videos in the posts it finds on X.    |
+------------------------------------------------------------+
```

- Codex and Claude Code show **Web Search** only. Grok Build shows all
  four, **Image Understanding** and **Video Understanding** among them: its
  searches look at the images in the pages and posts they find, and its X
  search watches their videos, which xAI leaves off unless asked. A switch
  whose searches are off stays in its place, disabled.
- The vendor's other built-in tools, such as xAI's code execution and image
  generation, stay off: Demi has its own.
- The search goes with every request of the entry's models, as the Codex
  CLI sends it: every model each catalog lists today can search.
- An agent picks up a change at its next request, since a session builds its
  runtime again when its entry changed.
- API key entries are out of scope.

### The conversation

A search is a step of the run, beside commands, with the vendor's word for
what it did:

```text
v Searched the web 2 times, ran 1 command, for 6 s
    (globe) Web search  rust latest stable release
    (globe) Open page   blog.rust-lang.org/releases/latest
    $ cargo --version
  The latest stable Rust is 1.99.1. [Rust release notes](https://…)
```

- A step's line is what it did, worded the same while it runs and once it
  ended, then its words: **Web search** and its queries, **Open page** and
  **Find in page** and the page, **X search** and **X people search** and
  the query, **X post** and the post, **View image** and **X video** and
  what it looked at. A globe marks the web's, an at sign X's, a picture an
  image looked at, and a clapperboard a video watched.
- While the vendor runs it, the step shows as running, like a command.
- Expanding a step lists the sources the vendor returned for it, each a
  title and a link; a step without any shows its words alone.
- The run's line counts the searches as it counts commands: *searched the web
  2 times*, *opened 1 page*, *searched X 3 times*.
- A source without a title, as an X post comes, shows its address.
- The vendor's citations are in the answer's text already, as Markdown links
  that each vendor writes; Demi adds none.

### What a model reads afterwards

- **The same vendor's model** gets each search back as the vendor sent it,
  so it remembers what it found without searching again. Grok's X search
  results come back only this way: its records hold them encrypted.
- **Another vendor's model** gets none of it, as it gets none of another
  vendor's reasoning. The answers that cited the searches, with their links,
  stay in the transcript.
- **The switch turned off**: the vendor's records are left out too, since a
  vendor can refuse a search record without its tool.

### Design

- Stored data: the control database's `providers` gains `web_search`,
  `x_search`, `search_images` and `search_videos`, on, by migration: an
  entry that existed searches as a new one does.
- `provider-registry`: `ProviderEntry` gains the four switches, and a
  family says the tools its vendor has (`ProviderFamily::searches`): Codex
  and Claude Code the web, Grok Build the web and X, with image and video
  understanding. `FamilyArgs.searches`
  carries those the entry turned on to the provider it builds.
- `web-api-protocol`: `ProviderDto.search` holds each switch, null for a
  search the vendor does not have; `ProviderPatch.search` sets them, and a
  switch of a search the vendor does not have is refused.
- `shared-types`: `OpaqueView::Search { action, sources }`, with
  `SearchAction`: `web { queries }`, `open_page { url }`,
  `find_in_page { url, pattern }`, `x { query }`, `x_people { query }`,
  `x_post { url }`, `image { url }`, `x_video { url }`, and `SearchSource`
  (`url`, `title`).
- `provider-common`:
  - `wire::responses` maps `web_search_call` (OpenAI's and xAI's shape, with
    the sources its action names) and xAI's `custom_tool_call` of
    `x_keyword_search`, `x_semantic_search`, `x_user_search` and
    `x_thread_fetch` to search steps. The encrypted reasoning the proxy
    writes for each server call is an entry of that call's step, matched by
    id (`tco_<id>` for `ws_<id>` or `ctc_<id>`), not thinking; a reasoning
    item that matches no call is the model's. None of these is a call Demi
    runs.
  - `ResponsesDialect.searches`: a recorded search of a kind the request
    does not declare is left out.
- `provider-codex`: adds `{"type": "web_search", "external_web_access":
  true}`, and includes `web_search_call.action.sources`.
- `provider-grok-build`: adds `{"type": "web_search"}` and
  `{"type": "x_search"}` by their switches, with
  `enable_image_understanding` and, for X, `enable_video_understanding` by
  theirs; their `view_image` and `view_x_video` calls are search steps too.
- `provider-claude-code`: passes `--tools WebSearch` instead of
  `--tools ""`. The CLI runs the search itself; the run maps the
  `WebSearch` call to a running step and its result's `tool_use_result`
  (the query, and each result's title and URL) to its end, and the session
  mirror keeps both the call's line and the result's on the step's block.
  - A search the model called beside Demi's tools ends in a later request
    of the turn, so a step is found by its call id across the turn's
    requests, and one still running when a request ends without its
    response ends as failed.
  - A search of a stream the CLI abandons for a complete answer ends as
    failed: the CLI never runs it.
  - With the switch off, the session the CLI resumes leaves out the
    searches' lines, as the Responses providers leave out their records.
- `web-ui`: the settings section and the `search` view's line and sources;
  `web-gallery`: their specimens, with searches that ended, failed and run.

## Spike

| Question | What ran | What it showed |
| --- | --- | --- |
| Grok Build's web and X search on a subscription | Responses requests with `web_search` and with `x_search` | `web_search_call` with the query and sources; X search as `custom_tool_call` items, each followed by an encrypted `tco_…` reasoning item; answers cite with `[[1]](url)` |
| Grok's memory of an X search | The output items replayed, and a question about a post | Answered from them without searching |
| Claude Code's WebSearch on a subscription outside the US | `claude -p --tools WebSearch` | The CLI searched; `tool_use_result` holds the query and each result's title and URL; the answer ends with sources as links |
| Codex's web search | `codex exec` with `web_search = "live"` | A `web_search` item with the query and each result's title, domain and snippet; the answer cites with a Markdown link |
| Grok's look at a post's video | `x_search` with image and video understanding, asking what a post's video shows | Two `view_x_video` calls, and an answer that describes the video |

## Prior art

- Codex CLI adds the hosted tool to every request
  ([`hosted_spec.rs`](https://github.com/openai/codex/blob/02ca1e0d990e496366bda7c30e2981cf296783ce/codex-rs/core/src/tools/hosted_spec.rs#L14-L45))
  and shows each action as a history cell: *Searched*, *Opened*
  ([`search.rs`](https://github.com/openai/codex/blob/02ca1e0d990e496366bda7c30e2981cf296783ce/codex-rs/tui/src/history_cell/search.rs#L10-L35)).
- Claude Code's WebSearch is a client tool that makes a nested request with
  Anthropic's `web_search_20250305`. Zed's Claude Code adapter shows its
  results as titles and links
  ([`web.ts`](https://github.com/zed-industries/claude-code-acp/blob/52c241e71512ec19c4dd79c95c51ce67f04d7aac/src/tool-calls/reporters/web.ts#L23-L55)).
- CLIProxyAPI sends `x_search` to the Grok Build proxy and filters xAI's
  `x_*` server calls so that a client never runs them
  ([`xai_executor_response.go`](https://github.com/router-for-me/CLIProxyAPI/blob/90c447d3a8727d115aa7552ec93ce023736ed492/internal/runtime/executor/xai_executor_response.go#L110-L175)).

Demi searches by default, as the Codex CLI and Claude Code do: each search
spends the subscription the user pays for, and a user who wants an agent not
to spend it turns the entry's switch off.

## Alternatives

- **A search as a `ToolCallBlock`.** It would draw like a command at no
  cost, but every provider replays a tool call as a call of a tool, which
  another vendor's model would see as a tool it does not have, and the agent
  would treat it as a call to run.
- **One switch in Settings › Web Search** for all three. The searches belong
  to different subscriptions, with different quotas and capabilities; the
  provider's page is where the user decides what an entry spends.
- **A `demi x search` command** that asks Grok once. A model of another
  vendor reaches X search through a Grok subagent instead.

## Open questions

None. The switches are on from the start, and Codex searches
live (`external_web_access: true`), for current results.
