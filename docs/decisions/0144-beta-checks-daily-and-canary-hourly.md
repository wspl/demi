---
status: proposed
---

# Beta looks for a new release every day, Canary every hour

Supersedes: 0125 (A server prepares its next release ahead), the hourly
check on both channels.

## Problem

Alex runs a server on Beta. Beta takes a release only when someone promotes
a Canary that served well, a few times a month, yet the backend asks GitHub
for Beta's newest release every hour, 24 requests a day that almost always
find nothing new. Canary, which publishes every push to main that passes, is
the channel an hourly check is for.

## Decision

The backend looks for the newest release of its channel at the channel's
cadence: every hour on Canary, every day on Beta. Nothing on screen changes:
Settings › Updates says when the server last looked, as it does.

- The cadence counts from the end of the last check, whichever made it: a
  start, a change of channel, **Check Now** or the cadence itself. So Alex,
  who switches from Beta to Canary, gets the switch's check at once and the
  next an hour later, not when Beta's day would have ended; a switch back
  waits a day from its own check.
- A check that comes due while another runs waits for it, then checks only
  if it is still due, so a switch and the cadence never check twice in a
  row.
- A failure of the work ahead is tried again at the next check, on Beta a
  day later; **Check Now** tries at once, as today.

## Spike

None.

## Prior art

Firefox sets `app.update.interval` per channel: its builds check less often
on release than on Nightly, which Mozilla shortened for Nightly so testers
take each build soon after it lands
([bug 630880](https://bugzilla.mozilla.org/show_bug.cgi?id=630880),
[Checking For Updates](https://wiki.mozilla.org/Software_Update:Checking_For_Updates)).
Demi follows the per-channel cadence and keeps a channel's checks below
GitHub's 60 unauthenticated requests an hour per address.

## Alternatives

- **Every hour on both,** as before: Beta would ask 24 times for what
  changes a few times a month.
- **Every day on both:** a Canary fix would wait up to a day, and Canary is
  for having fixes as they land.

## Open questions

None.
