---
status: proposed
---

# A shell call holds until its command ends, and what arrives moves it aside after 10 seconds

Supersedes: 0052 (A shell call waits for its command, and a command wakes
the agent only when it ends), the part that a call returns after 30
seconds; and 0077 (A report waits for a running call, as a steer does),
the part that only the user's send now returns a call early.

## Problem

An agent starts a test run that takes six minutes and wants its result
before it goes on. After 30 seconds the call returns with the command in
the background. The agent runs `sleep 120; demi shell status 17`, which
also returns after 30 seconds, still running; so it runs another, and
another: nine calls in six minutes, each a request that carries the whole
conversation, and each of those sleeps wakes the agent once more when it
ends.

The agent waits in its turn because its model was trained where a command
holds its call until it ends, two minutes by default and ten at most. The
system prompt says to wait by ending the turn, and the agent does not.
0052 refused a script that starts with a long sleep; that refusal is gone,
and it taught the agent only after a call it had already spent.

A user's steer waits for the running call, up to 30 seconds today. A call
that holds longer would make it wait longer.

## Decision

**A call holds until its command ends, 10 minutes at most.** Waiting in
the turn is then what it looks like: `sleep 120; demi shell status 17`
holds for 120 seconds once, and `while …; do sleep 10; done` holds until
its loop ends. A command still running after 10 minutes moves to the
background and wakes the agent when it ends, as today.

**What arrives during a call moves its command to the background once the
command has run 10 seconds:** the user's steer, an agent's message, the
end of a command in the background, and the return of another call of the
same response, each the same way. One that arrives
earlier waits until the command ends or reaches 10 seconds, whichever
comes first, so a quick command still returns whole in its call. The
user's send now moves it at once, as today. What arrived then joins the
turn after the call's result, as it does at any boundary.

10 seconds is the longest a message waits for a call: long enough that
most commands end within it and return in one call, short enough that the
user's steer reads as sent.

So the calls of one response race: a test run of six minutes beside a
`git status` moves to the background once the status has returned and the
run has reached 10 seconds, and both results go to the model together. The
model's next request needs every call's result, so a call cannot return
alone; the others move aside instead.

The model reads:

* the shell tool, which names no limit, so the agent plans no way around
  one; a result that moved says why when it happens: `Run a shell script
  on the conversation's Host. The call returns the output when the script
  ends; a command that moves to the background wakes you when it ends.
  service: true runs a command until it is stopped, such as a server: the
  call returns once its output is quiet for 2 s, and only its exit wakes
  you. Calls in one response run in parallel.`
* a call returned for what arrived: `[A message arrived, so command 17
  moved to the background. It keeps running; its end wakes you.]`, in
  place of the one that names the user's message.
* a call returned after 10 minutes, as today: `[The command is still
  running, so it moved to the background; its end wakes you.]`
* a command in the background that prints nothing for 10 minutes still
  wakes the agent once, and its report says so; the tool no longer
  announces it.

The guideline `To wait for a command or a subagent, end your turn` stays:
it still holds for a subagent and for a command past 10 minutes.

The design to build from:

* `agent-tools`: `CALL_WINDOW_MS` becomes 600,000, and a new
  `ARRIVAL_WINDOW` of 10 seconds says its reason beside it. A call's watch
  ends when its command ends, when its window passes, on the user's send
  now, or once the command has run `ARRIVAL_WINDOW` and something waits for
  the next boundary then; a steer the user took back counts for nothing.
  Neither time runs while the command waits for the user's permission,
  and each starts again once it goes on. `Moved` gains `Input` and `Call`
  beside `SentNow`. A `service` call keeps its window, quiet for 2 seconds
  or 30 seconds, and what arrives moves it as it moves any call.
* `agent-session`: `SendNowArrival` becomes `Arrival`, which tells a call
  both the user's send now and that an input waits for the next boundary:
  a steer, an agent's message, a command report, counted from the
  round's request, or another call of the response that has returned.
* Nothing stored changes; the web page, the runner and the protocols do
  not change. The steer's line, `Sends at the agent's next step`, stays
  true.

## Spike

None.

## Prior art

* Claude Code's Bash tool holds a call until its command ends, two
  minutes by default and ten at most. A message the user sends meanwhile
  waits until the call ends; `Ctrl+Enter` moves the running command to the
  background and hands the message over at once
  ([interactive mode](https://code.claude.com/docs/en/interactive-mode)).
  Demi does that by itself after 10 seconds, so the user never has to know
  the key.
* Codex returns from `exec_command` after 10 seconds by default, and its
  models are trained to read again.

## Alternatives

* **Refuse a script that starts with a long sleep, as 0052 did.** The agent
  learns only after a call it spent, and a loop that sleeps passes.
* **Tell the model in the prompt or in the result.** The prompt says it
  already, and the agent sleeps anyway.
* **Let what arrives wait for the call's end.** A steer would wait up to 10
  minutes.
* **Move the command aside the moment something arrives.** A command that
  would end a second later costs the agent another step and another wake.
