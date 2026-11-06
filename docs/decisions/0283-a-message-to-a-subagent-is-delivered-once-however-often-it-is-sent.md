---
status: proposed
---

# A message to a subagent is delivered once, however often it is sent

Supersedes: 0220 (An integration drives its conversations as the web app
does), the part that says a subagent's message takes no id of the program's.

## Problem

A reminder app built on Demi wakes a subagent when a reminder comes due. It
calls `POST /v1/conversations/{id}/agents/{agent}/messages`. Demi takes the
message, but the connection drops before the answer arrives, so the app sends
the message again. The route takes only `text` and Demi makes a new id for
each call, so the subagent receives two messages and works twice. The app can
stop retrying, and then a network failure loses the reminder instead.

The same app can retry a message to the main agent safely, because
`POST /v1/conversations/{id}/messages` takes an `id`. 0220 left it out here
because `demi agent resume` names no message. That is true of the command and
does not bind the route.

## Decision

The route takes the `id` the main agent's route takes, and answers as that
route does:

```rust
pub struct SubagentMessage {
    /// The id it takes, which a retry repeats so nothing is sent twice; one
    /// is made when none.
    pub id: Option<String>, // 1 to 64 characters
    pub text: String,
}
```

It answers `200` with `MessageAnswer { id }` instead of `204`, so a program
that named no id still learns the one made for it.

What a call does depends on the subagent and on what it holds:

| The subagent | It holds the id | What the call does |
| --- | --- | --- |
| Live | In its history or queue | Nothing; answers the id. |
| Live | No | Hands it the message, as today. |
| Closed | In its history | Nothing; the subagent stays closed; answers the id. |
| Closed | No | Resumes it with the message under that id. |

An id belongs to one agent: the same id sent to two subagents is two
messages. A repeated id with other text is the same message, and the first
text stands, as on the main agent's route.

A message that a stop left unread is in no history. A retry of it resumes the
subagent, which then reads it for the first time, so it still runs once.

### What changes in the code

Nothing new is stored. The history already holds each id: a message that opens
a turn as its user block's turn, and a steer that joins a turn as its block's
id. No migration is needed, and neither the web app's protocol nor
`demi agent resume` changes.

* **A live subagent.** Its session recognizes a steer it already holds, after
  a compaction too, as it does for the main agent, so a repeated id joins
  nothing.
* **A closed subagent.** `Earlier::read` over its `session_store` answers
  whether its history holds the id, as a steer's block or a user block's turn.
* **Two equal calls at once.** A closed subagent's message is a new
  `StartInput::Message`, which runs in the tree's `starts` queue, one start of a
  parent's children at a time. Today, two calls that both find the subagent
  closed end with the second refused as "still running". In the queue, the
  second finds the subagent live, holding the id in its queue, and answers the
  id; or, if it ended during the wait, finds the id in its history. Each check
  runs in the queue because a check before it can be stale when the turn comes.
* **A resume under an id.** `reopen` takes the queued message whole, so the
  message keeps the program's id. `demi agent resume` makes a new id, as today.

A server that restarts between saving the resume and answering restores the
subagent with the message in its checkpoint's queue, so the retry finds it.

The guide and the OpenAPI document say what the route now takes and answers.

## Spike

None.

## Prior art

* [OpenCode's server](https://opencode.ai/docs/server/) takes an optional
  `messageID` on `POST /session/:id/message`. Its documentation does not say
  what a repeated id does.
* [Stripe](https://docs.stripe.com/api/idempotent_requests) takes an
  `Idempotency-Key` header and saves the first response for retries. Demi saves
  no response: the answer is only the id, which the retry already holds, and
  the history is the record of what arrived.

## Alternatives

* **A table of the ids seen.** It would store a second time what the history
  holds, and it would need its own expiry.
* **An `Idempotency-Key` header.** The main agent's route names the same thing
  `id` in its body, and one concept has one name.
* **Only the live path.** It is two lines, but a retry that arrives after the
  subagent finished would resume it and run the work again, which is the case
  this decision exists for.

## Open questions

None.
