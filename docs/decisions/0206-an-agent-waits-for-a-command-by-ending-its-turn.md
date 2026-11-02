---
status: proposed
---

# An agent waits for a command by ending its turn

Supersedes: 0201 (Claude Code hands the model Demi's messages as written),
the three lines the model reads when a command moves to the background; and
0113 (A shell call holds until its command ends, and what arrives moves it
aside after 10 seconds), the guideline on waiting that it kept.

## Problem

An agent starts a build that takes four minutes. A subagent's message
arrives, so after 10 seconds the build moves to the background, and the
result says `[A message arrived, so command 17 moved to the background. It
keeps running and its end wakes you, so if nothing else needs doing, end your
turn.]` The agent reads the message, still wants the build's output, and
runs `sleep 20; demi shell output 17`, then again, or one
`while demi shell status 17 | grep -q running; do sleep 5; done` that the
next message moves aside too. Each look is a request that reads the whole
conversation again: on one server over eight days, 1,170 such calls read
about 270 million input tokens, mostly from the prompt cache. The turn stays
open, so the user gets no answer while the agent waits.

Moving the command aside is not the fault: the agent gets the choice of
looking into a command that runs longer than it should, such as a `find /`
typed by mistake, and stopping it. Over the same eight days, of 2,529
commands moved aside, the agent left 62% alone and stopped 9%; it came back
to the rest, 81 of them two times or more.

The agent's reasoning shows two beliefs. It holds that ending the turn
before the command ends would be premature, as if the turn were the task.
And it does not count a sleep before a look as waiting: told only "Do not
wait for it", it waited as often as before.

## Decision

The model reads that the end of what it waits for wakes it and it goes on
then, and that a sleep or a loop is not how it waits.

The guideline of the system prompt becomes:

```text
- To wait for a command or a subagent, end your turn: its end or its message
  wakes you, and you continue then. Never wait with sleep or a loop.
```

A command that moves to the background says:

- `[A message arrived, so command 17 moved to the background. It keeps
  running and its end wakes you. Do not wait for it: do other work
  meanwhile, or tell the user what is running and end your turn.]`
- `[Another call returned, so command 17 moved to the background. It keeps
  running and its end wakes you. Do not wait for it: do other work
  meanwhile, or tell the user what is running and end your turn.]`
- `[The command is still running, so it moved to the background. Its end
  wakes you. Do not wait for it: do other work meanwhile, or tell the user
  what is running and end your turn.]`

A single look at a command, `demi shell status 17`, stays what it is: the
way to find out whether the command goes as it should.

The guideline is `HARNESS_GUIDE` in
`crates/backend-user-shard/src/conversation/product.rs`; the lines are in
`crates/agent-tools/src/result.rs`. Nothing stored changes.

## Spike

Moments where a command of a Claude Opus 5.5 agent moved to the background
were rebuilt from one server's transcripts as requests to the vendor's API,
as Demi replays a transcript, thinking left out, with today's system prompt.
Ten moments where the agent most often waited were each sent three times per
wording, and every wording differed from the others only in its text.

| Wording | Calls that waited |
| --- | --- |
| Today's guideline and lines | 23 of 30 |
| Lines that say only "Do not wait for it" | 22 of 30 |
| A guideline that names sleep and loops, today's lines | 9 of 30 |
| A guideline without "Never wait with sleep or a loop" | 23 of 30 |
| This decision's guideline, lines without "Do not wait for it: ..." | 12 of 60 |
| This decision | 8 of 60 |

- Without "Do not wait for it: ...", two answers ended the turn with no
  text. With it, none did, and the agent said what was running, such as
  "The test suite is still running; once it ends I will go through the
  failures."
- Fifteen moments where the agent had stopped the command, looked at it
  once, done other work or ended its turn were sent three times each with
  today's text and with this decision's. The agent stopped the command in
  14 of 21 and 14 of 21 of the first kind, and the other kinds kept their
  shares.
- The waits left were on a file the command writes rather than on the
  command, a sleep beside a note to the user, a sleep before answering a
  question that needs the output, and a loop on the command's status. How
  often they happen across all moved commands is for the server's data to
  show after this lands.
- A wording that refused a call that only waited, as Claude Code does, also
  worked, but costs the agent a refused call each time.

## Prior art

- Claude Code (2.1.316) tells its model, for a command run in the
  background, "You will be notified when it exits — do not poll its output
  file while waiting", and refuses a foreground `sleep`: "To wait for a
  condition, use Monitor with an until-loop … Do not chain shorter sleeps to
  work around this block." Demi says the same in its guideline and refuses
  nothing.

## Alternatives

- **Refuse a call that only waits.** The agent then goes on, but each wait
  costs it a refused call first.
- **Let only the user's messages move a command aside.** Fewer moves, but
  the agent loses the choice to look into a command that runs long, and
  still waits on one that passes 10 minutes.
- **Tell the model only in the line, not the guideline.** The guideline
  holds for every wait, a subagent's as well, and the line alone left more
  waits.

## Open questions

None.
