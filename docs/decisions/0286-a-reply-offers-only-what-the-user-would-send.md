---
status: proposed
---

# A reply offers only what the user would send

Supersedes: 0182 (A reply offers the user's likely next messages), the kinds of
a suggested reply, the new topic, and the instruction.

## Problem

An answer ends with "Shall I merge it?". The row offers "Merge it", "Merge it
now" and, under a light bulb, "How do I check it on the server?". The first two
do the same thing, and the third is work nobody asked for. Another answer only
says "Tell me if you want the title smaller too", and still offers three pills.
A third asks which file failed, and a pill answers for the user: "I did not see
that file".

Asking for up to three makes the model fill the row. The rule that decides
whether to offer anything is one clause among others, and a small model passes
over it. Only a repeat of the very same words is dropped, and nothing says what
to do when an answer puts several decisions at once, so each pill answers part.

## Decision

The row offers only what the user is truly likely to send. One pill is normal,
and none is common.

```text
  The fix is in and the tests pass. Shall I merge it?
  [copy] [fork] [share]  2 minutes ago
  ( <- Merge it )
```

- The new topic goes, with its light bulb. Every pill starts with the reply
  arrow, so the pills look as answers do today.
- `SuggestedReply` and `SuggestedReplyKind` go, and `replies` becomes a list of
  strings. A step of the control schema rewrites what is stored, dropping new
  topics.
- The row still holds at most three pills, and keeps its height as 0182 says.

The instruction first asks whether the answer waits for the user's decision. It
answers exactly `NONE` when the answer only reports, explains, advises or
confirms; says what the agent does next by itself; only mentions what it could
do if the user wants it; or asks the user to do something, or for facts only
the user has. Otherwise each candidate starts with its stance:

| Stance | What it does | When |
| --- | --- | --- |
| `yes:` | Approves what the answer proposes | Once at most |
| `no:` | Declines it or holds it off | Once at most, only when the answer gives a real reason to, such as a cost, a risk or a condition to wait for |
| `pick:` | Takes one of the options the answer names | One line for each, in place of `yes:` |

It adds these rules:

- One candidate is normal, and none is written just to offer more.
- Each candidate makes the agent do something different from the others.
- When the answer puts several decisions, each candidate settles all of them,
  or the model answers `NONE`.
- A candidate is a decision, never a statement of fact about the user.

The reply is read in code, which holds what a model may not: it keeps the first
`yes:` and the first `no:`, and drops a line without a stance. The stance stays
in the stored response (0285) and does not reach the page.

An older page refuses a conversation's metadata that holds a list of strings, so
it shows none of the metadata's changes until it reloads to the new version, as
every page does after an update.

## Spike

Each set of turn ends was replayed on the small model that serves these requests
today, with the request as the backend builds it, from one user's conversations
of four days. A larger model judged each result against one rubric, and samples
of its judgments were read by hand. The instruction was written while reading
the first set, so the second, which was not read, is the one that counts.

| 198 turn ends not read before | Today | This decision |
| --- | --- | --- |
| Turn ends with pills | 98 | 76 |
| Pills in all | 273 | 100 |
| Rows of one pill | 1 | 55 |
| Two pills that do the same | 24 | 0 |
| A pill that invents a fact or new work | 16 | 1 |
| Pills that each settle part of the decisions | 17 | 2 |
| Pills where the answer waits for no decision | 16 | 4 |
| No pill where a few words could decide | 0 | 5 |
| Rows with none of these faults | 36 (36%) | 69 (90%) |
| Rows holding the user's next message, or close to it | 10 | 10 |

The change removes noise and predicts no better: a third of the pills reach the
same 10 messages. Of 49 new topics in the first set, none matched what the user
sent next.

The judge is a model and the replay is not what the user saw, so the numbers
compare the two instructions and no more. The rate of use that the table of 0285
keeps decides whether this worked. The request costs what it did.

## Prior art

- [Open WebUI](https://docs.openwebui.com/features/chat-conversations/chat-features/follow-up-prompts)
  generates follow-up prompts after each response with its task model. They are
  new questions to ask, the kind this decision drops: a coding agent's user
  mostly answers the agent.
- OpenCode's [opencode-ghost](https://www.npmjs.com/package/opencode-ghost)
  answers `NONE` when no reply is clearly right, which this decision makes the
  first question.
- Claude Code and Codex offer one suggestion at a time (0182), the form the
  usual row of one pill now takes.

## Alternatives

- **A count of two or three with better wording.** Asking for more than one is
  what makes the model invent the second.
- **`yes:` and `no:` on every row.** An early version did this: faults fell as
  far, but most rows held the same plain pair of approve and refuse, which a
  user can type or ignore.
- **Comparing the pills' words in code.** "Merge it" and "Go ahead" share no
  words, and a rule of thumb for each language would be wrong in the others.
- **A larger small model.** Each turn would cost more, and the fault lies in what
  the instruction asks.

## Open questions

None.
