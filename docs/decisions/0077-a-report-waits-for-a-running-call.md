---
status: proposed
---

# A report waits for a running call, as a steer does

Supersedes: 0052 (A shell call waits for its command, and a command wakes
the agent only when it ends), the part that a message arriving during a
call returns the call early; and 0071 (A call that waits for the user's
permission holds until the user allows it), the part that what arrives
while a call waits for the user returns it once the user has decided.

## Problem

An agent waits for a download with `sleep 180; demi shell output 17`.
After 30 seconds the call goes to the background, and its end will report
later; the agent waits again. A report arrives, the end of an earlier
wait, and returns the new call after a second or two with
`status: running, output: (empty)` and no word of why. The agent waits
again, and each wait it starts becomes a report that cuts a later one
short, so the waits feed each other.

A call returns early whenever a command report or another agent's message
arrives, though a user's steer, which matters more, waits for the call.

## Decision

**A command report and an agent's message that arrive during a call join
the turn at its next boundary, after the call's result, as a user's steer
does.** Of what arrives during a call, only the user's send now returns
it. A parent's message reaches a busy child when the child's call returns,
as a steer reaches the agent.

**A call that returns while its command runs says why, and that the
command's end wakes the agent:**

* `[The command is still running, so it moved to the background; its end
  wakes you.]`, where it said `[The command is still running, so it moved
  to the background.]`
* `[The user sent a message, so command 17 moved to the background. It
  keeps running; its end wakes you.]`, where it said `… It keeps
  running.]`

These say what becomes of this command, which the tool's definition says
in general.

**What else ends a call before its command ends is the user's or an
agent's own act, and stays:** the user's Stop, a Cloud reset, a
conversation's deletion, a detach, and a parent's `demi agent abort` of
its child. A backend that stops ends its calls too, and their commands
report when it is back.

The design to build from:

* `agent-session`: an agent message and a command report no longer end a
  call's window; only the user's send now does, and the names of what
  tells a call's window of it say send now (`SendNowArrival`).
  `WindowEnd` is removed.
* `agent-tools`: a call's watch ends when its command ends, when its window
  passes, or on the user's send now, as `Moved` already names. Its entry
  comment says that a steer, an agent's message and a command report are
  all read at the next boundary.
* A call that waits for the user's answer (0071) holds as before; what
  arrives meanwhile joins the turn after its result.
* Nothing stored changes.

## Spike

None.

## Prior art

* Demi's own steer: a message the user sends during a call joins the turn
  at its next boundary, and only Send Now interrupts the call.

## Alternatives

* **Keep returning the call, and say why.** The agent would know that its
  command still runs, but each report would still cut its wait short and
  cost it a step.
* **Return the call for an agent's message, not for a report.** A parent's
  message is no more urgent than the user's steer, which waits.

## Open questions

None.
