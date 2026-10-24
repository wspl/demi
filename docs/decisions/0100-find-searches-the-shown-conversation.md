---
status: proposed
---

# Find searches the shown conversation

## Problem

Alex has a long conversation open and wants the message, written days ago,
where the agent named the port it chose. Find, ⌘F, in the conversation opens
the search window, which searches every conversation. It answers with one row
per conversation, each with its newest matching message, so the conversation
Alex is in shows one line at most, and often lies under others that also say
"port". The browser's own find, which ⌘F replaced there, sees only the
messages the page has loaded, not the older ones further up.

## Decision

**Find in the conversation area opens the search window scoped to the shown
conversation.** The field starts with a token, *This conversation*, before
the text, as Slack's ⌘F in a channel starts its search with that channel:

```text
+--------------------------------------------------------------+
| [ This conversation x ] port|                            (x) |
+--------------------------------------------------------------+
| You                                               3 days ago |
|   Which **port** does the dev server use?                    |
| Agent                                             3 days ago |
|   It listens on **port** 4100, set in .env.                  |
| You                                                 1 hr ago |
|   Change the **port** to 4200.                               |
+--------------------------------------------------------------+
```

* Each row is one message of the conversation that holds every word typed,
  case ignored, as the search window matches today: the user's messages and
  the agent's answers, not a subagent's text or a command. The row shows who
  wrote it, *You* or *Agent*, when, and the line around the first match,
  the matches marked. The newest come first.
* ↑ and ↓ move the selection; Return or a click scrolls the conversation to
  that message and closes the window, as a result of the search window
  opens its conversation at the message today.
* The token's ✕, or Backspace at the start of the field, removes it. The
  window then searches every conversation with the same text, titles
  included, exactly as ⌘K's search does. The token does not come back until
  the window opens again.
* While the field is empty, the scoped window lists nothing and says *Type
  to search this conversation*.
* Find opens the window without the token where no conversation is shown,
  such as a new conversation's empty page. ⌘K and Search in the sidebar
  never add the token.

The design to build from:

* `web-api-protocol`: `GET /api/conversations/{id}/search?q=` answers
  `MessageResults { messages: Vec<FoundMessage> }`, where `FoundMessage`
  holds `block_id`, `author` (`user` or `agent`), `time`, and the line and
  its ranges as `SearchMatch` holds them today. It answers at most
  `RESULTS_MAX` messages, the newest, and says `limited` when there were
  more; the window then ends its list with *Showing the 50 most recent
  matches*. The query is the same `SearchText` as `GET /api/search`, and a
  conversation the caller cannot read answers 404.
* `backend-database`: `SearchIndexes::find_in`, which `demi conversation
  history search` reads, answers each text's block, kind, time and text
  beside its position, so one query serves both. The route cuts each text to
  its line with the same `match_line` the global search uses.
* `web-ui`: `SearchDialog` takes `scope: { label: string } | null` and a
  second source, `searchIn`, for the scoped list; `useConversationSearch`
  keys its answers by the scope as well as the text, so removing the token
  searches again. The gallery's search specimen gets a scoped state whose
  token removes and whose rows open, saying so in a toast.
* `web`: Find in the conversation area opens the window with the shown
  conversation's id; Return calls `openAt` with the row's block.

An older backend answers the new route 404, and the window shows its failure
with Retry; the global search is unchanged.

## Spike

None: the index already holds every message's block, time and text, the
history command already searches one conversation through it, and the
search window already opens a conversation at a block.

## Prior art

* Slack: ⌘F in a channel or a DM opens search with an `in:` modifier for it,
  which the user removes to search everywhere
  ([Search in Slack](https://slack.com/help/articles/202528808-search-in-slack)).
* Discord: search in a DM or a channel starts with an `in:` filter chip,
  removed with Backspace.
* Telegram and WhatsApp: search in a chat lists the chat's matching
  messages, newest first, and opening one scrolls to it.

Demi departs from Slack in showing the scope as one token with ✕ rather than
text the user edits, since Demi has no other modifiers to type beside it.

## Alternatives

* **A find bar inside the conversation, with next and previous, as a
  browser's.** The conversation loads its messages as the user scrolls, so
  the bar would need the backend's search anyway to reach older ones, and
  would be a second search beside the window.
* **The browser's own find.** It sees only the loaded messages, and the
  user accepted giving it up when ⌘F became Find.
* **A scope menu, All or This conversation, beside the field.** One more
  control to read on every search, where Slack and Discord show the scope
  only when there is one.

## Open questions

* Should the conversation's ⋯ menu also have *Find in Conversation*, for a
  phone, which has no ⌘F?
* Is *Agent* the right label for the agent's answers in the rows? The
  conversation itself labels neither side.
