---
status: proposed
---

# Every suggested replies answer is kept, with what the user did with it

Supersedes: 0194 (Suggested replies outlive a restart), the table of one row
per conversation.

## Problem

An answer ends with "Shall I merge it?" and offers "Merge it" and "Merge it
now". Whether pills like these help nobody can check: the next answer's replies
overwrite the row, the log says only `kept` or `none`, and a tap sends an
ordinary message. Only the latest
replies of each conversation exist. Whether a change of the instruction helps
can only be guessed by replaying old turns, which shows what the model might
write now, not what the user saw.

The log misleads as well. It names the conversation's model, so a request
answered by a small model is logged under the large one.

## Decision

Each answer of a suggested replies request is a row that stays. What the
conversation offers is its newest row, so one table owns the fact and no second
copy can disagree.

```sql
CREATE TABLE conversation_suggested_replies (
  id              INTEGER PRIMARY KEY,
  conversation_id TEXT NOT NULL COLLATE NOCASE
                    REFERENCES conversations (id) ON DELETE CASCADE,
  block_id        TEXT NOT NULL,
  answered_at     INTEGER NOT NULL,
  -- What answered, the instruction's hash and the model's whole answer;
  -- null on a row an earlier release kept.
  provider_id     TEXT,
  model           TEXT,
  instruction_hash TEXT,
  response        TEXT,
  replies         TEXT NOT NULL,
  -- The reply the user last took, its place in replies, and how.
  used_reply      INTEGER,
  used_as         TEXT CHECK (used_as IN ('sent', 'edited')),
  used_at         INTEGER
) STRICT;
CREATE INDEX conversation_suggested_replies_latest
  ON conversation_suggested_replies (conversation_id, id);
```

- A request that was stopped or failed has no answer and no row; the log says
  so, as it does today.
- `instruction_hash` is computed from the instruction's text, so a
  change of the text needs no number to be raised by hand.
- The request's message is not stored: the conversation's transcript holds what
  it was made from.
- Deleting a conversation removes its rows. There is no other limit: a row is
  under 1 KB, and 400 answers a day come to at most 150 MB a year.
- The step that makes the table carries over the existing rows, with their
  source columns null.

The page tells the backend when the user takes a pill, without waiting for the
answer. A failure is logged on the page and changes nothing the user sees, since
this only measures:

```text
POST /conversations/:id/suggested-replies/used
{ "blockId": "...", "reply": 0, "as": "sent" }
```

`sent` is a tap. `edited` is Edit, or a long press on a phone, which puts the
words into the composer. The latest of them stands.

`SmallRequests` resolves the small model apart from running the request, and
`suggested_replies.ended` logs the provider and model that answered.

An older page reads the metadata as before and reports no use until it reloads
to the new version, as every page does after an update.

## Spike

None. The design adds one table and one small request, and asks nothing of a
model.

## Prior art

- [Langfuse](https://langfuse.com/docs/prompt-management/features/link-to-traces)
  links each generation to the prompt version that made it and sums latency,
  cost and scores per version, the comparison this table allows. Demi keeps the
  few columns it needs in its own database, and takes the version from a hash.
- [Open WebUI](https://docs.openwebui.com/features/chat-conversations/chat-features/follow-up-prompts)
  shows follow-up prompts on the latest answer and, with a setting, keeps them
  on earlier ones. This record keeps them for measuring; the page still shows
  only the latest answer's.

## Alternatives

- **A second, append-only table beside today's row.** It stores the same replies
  twice, for two readers to disagree about.
- **Writing the replies to the log.** The log is cleaned up by age and cannot
  say which pill the user took.
- **Marking the sent message as a tap.** It covers `sent` only, and puts a
  measure into the transcript and the lane's protocol.

## Open questions

None.
