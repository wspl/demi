---
"@demicodes/shell": minor
---

Registered commands can declare `runningHint`, replacing the generic "check again with shell_status, or call yield" line on running shell results while they are the foreground job. `demi agent` spawn and resume use it to tell parents to steer, abort, or end the turn and be woken on completion instead of polling.
