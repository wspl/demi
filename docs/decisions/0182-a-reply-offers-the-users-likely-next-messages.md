---
status: proposed
---

# A reply offers the user's likely next messages

## Problem

The agent ends a turn with "The fix is in and the tests pass. Shall I merge
it?". The answer the user gives most turns is a few words, such as "Merge
it", and to give it they click the composer, type the words and press
Return. On a phone that is a keyboard opened for two words.

## Decision

Once the main agent's turn has ended with an answer, a row of up to three
suggested replies stands under that answer's footer: each a pill holding a
message the user is likely to send next, written the way the user writes.

```text
  The fix is in and the tests pass. Shall I merge it?
  [copy] [fork] [share]  2 minutes ago
  ( <- Merge it )  ( <- Show me the diff first )  ( * Add a test for the old name )

  <- reply arrow: an answer       * light bulb: a new topic
```

- Each pill starts with an icon of its kind: a reply arrow for an answer to
  what the answer asked or offered (an approval, a choice among its
  options, a reply to its question), a light bulb for a new topic the user
  may raise next. The small model names each one's kind.
- A click on a pill sends its words at once as a new message, alone: the
  composer's draft and waiting comments stay as they are.
- Hovering a pill shows an Edit button at its right end, over the pill;
  Edit puts the words into the composer, after a draft already there, and
  gives the composer the focus. On a phone, a long press on a pill does
  what Edit does.
- The row belongs to the last answer only, while the conversation rests. It
  goes as the next turn starts, and an earlier answer never shows one.
- On a page that saw the turn end, the row's height is kept from the moment
  the answer's footer shows, while the suggestions are still being written
  and when none came, so nothing under the answer moves when they arrive. A
  page that opens a conversation already resting shows the row only when
  suggestions are kept for its last answer. The pills stay on one line: a
  long one ends in an ellipsis and its tooltip shows it whole; on a phone the
  line scrolls sideways.
- A turn that ended in an error, or that the user stopped, offers none; nor
  does a subagent, a shared page, or an archived conversation.
- Settings › Agent › Messages › Suggested Replies turns them off; it is on
  by default. Off, no request is sent and no row stands. The Small Model
  group's description says the model writes titles and suggested replies.

### How they are written

As the root rests after a turn that ended with an answer while its tree
ran, the shard sends one small request (`crates/agent-server/src/small_request.rs`) on the user's small
model, thinking off, under the new purpose `RequestPurpose::SuggestedReplies`
in the usage ledger, whose check the control schema's next step widens by
making the ledger's table again. A tree only opened, as for a page opening
an old conversation, asks for none. The request is not part of the session: the agent's
system prompt, tools and history stay out of it, and the answer itself is
untouched.

Its message holds:

- the user's last twelve short messages in the conversation, for how the
  user writes;
- the two exchanges before the answer, cut short;
- the answer, its last 5,000 characters.

Its instruction asks for zero to three candidates, one per line, the most
likely first, each marked `answer:` or `topic:`: each a complete, specific message (an approval, a choice
among the options the answer offered, an answer to its question, or the
obvious next instruction), never a vague one; only what the transcript
says, with no version, name or number it does not; "you" being the agent;
in the user's language, words and length; and exactly `NONE` when no
message is clearly likely. The reply is read line by line: an empty line or
`NONE` is dropped, as are numbering and quotes, a line over 120 characters,
and a repeat; a line without a mark is an answer. A turn that starts while the request runs cancels it, and a
result for an answer that is no longer the last is dropped.

The shard keeps each conversation's suggestions in memory with the answer
they follow, until its tree runs again; a restart of the backend forgets
them, which the next answer makes good. Every page of the conversation, on
any device, learns them from its metadata, as it learns the last turn's
speed:

```ts
interface SuggestedReplies {
  /** The text block of the answer they follow. */
  blockId: string
  /** At most three, the most likely first; empty when none was likely. */
  replies: { text: string; kind: 'answer' | 'topic' }[]
}
```

A pill's tap sends an ordinary message holding its words.

## Spike

Forty turn ends of a long coding conversation were run through Haiku with
this instruction, and each set of candidates was set beside the message the
user sent next.

- Every turn that ended asking for an approval or a choice, 14 of them, had
  the user's actual reply among its candidates, mostly in the user's own
  words.
- Turns where the user changed the subject or asked a question of their own,
  about 22, had none; nothing could have predicted them, and an unused pill
  costs nothing.
- Overall, about 40% of the turns had the reply the user sent.
- A first instruction without the user's recent messages wrote longer
  candidates in the agent's voice, once invented a version number the
  answer did not hold, and once swapped who was to act, writing "I" for the
  user's "you"; the recent messages and the rules on invented details and on
  "you" fixed all three.
- A second run asked each candidate's kind on fifteen of the turns: every
  candidate after an answer that asked for approval came back an answer, and
  new topics stood in two turns.

A request holds about 2,000 tokens, 3,000 at most, and answers 20 to 40.
At list prices that is about $0.0022 a turn on Claude Haiku 4.5 and $0.0007
on Gemini 2.5 Flash: for 200 answers a day, about $13 and $4 a month, well
under 1% of what the turns themselves cost.

## Prior art

- Gmail's [Suggested Replies](https://support.google.com/mail/answer/15464729)
  write a response to a received email that matches the tone and style of
  the user's past emails, the name and the voice Demi takes.
- Claude Code shows one suggested next prompt as dim text in the empty
  prompt box after an answer; Tab or Right Arrow places it, and Return sends
  a suggestion at once, which users asked to separate
  ([anthropics/claude-code#25787](https://github.com/anthropics/claude-code/issues/25787)).
- Codex's terminal UI added `tui.prompt_suggestions`, off by default
  ([openai/codex#47911](https://github.com/openai/codex/pull/47911)): one
  single-line suggestion of at most 240 characters, dim in the empty focused
  composer; Tab accepts it for editing, a separate Return sends it, and a
  result for a turn that is no longer the latest is ignored.
- OpenCode's [opencode-ghost](https://www.npmjs.com/package/opencode-ghost)
  plugin, at 1.3.13, asks a small model after each turn for the message the
  user would most likely send, from the recent transcript, with rules to
  copy the user's tone and to answer `NONE` when no reply is clearly right
  (`src/options.ts`, `DEFAULT_SYSTEM`). Demi's instruction started from its
  rules.

Demi departs from the coding tools in showing up to three, as chips under
the answer rather than as text in the composer, so a phone takes them with a
tap and a wrong first guess still leaves two; and in using the small model,
since a request on the conversation's own model spends a subscription's
limits on every turn.

## Alternatives

- One suggestion as dim text in the empty composer, taken with Tab: a phone
  has no Tab, and it shows one guess.
- The main agent writing the suggestions at the end of its answer, in a tag
  the page strips: no second request, but every answer carries an
  instruction that can shape it, the tag shows while it streams, and a weaker
  model leaves it out or writes it wrong.
- A request on the conversation's own model reusing its cache, as Claude Code
  does: better context, but a request on the main model every turn, which a
  subscription counts against its limits, and one Demi's command-line
  runtimes cannot fork cheaply.

## Open questions

None.
