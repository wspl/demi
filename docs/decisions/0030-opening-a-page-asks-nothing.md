---
status: accepted
---

# Opening a page shows what is known and asks nothing

## Context and Problem Statement

Some views start work when they appear and show its result when it ends:

* Settings › Providers waits for a catalog of vendors before it shows the
  providers the page already holds.
* A provider's usage limits ask the vendor again whenever they come into
  view.
* Settings › Skills checks every skill's source for updates whenever it
  opens.
* A device's latency and loss are measured, with twenty probes, whenever
  its page shows.
* A Claude Code installation is followed by reading its state every two
  seconds.

So these views change after they appear, and opening one costs a request to
a vendor, a Git server or the Host.

## Considered Options

* Keep the reads, and reserve room for their results.
* A view shows the last value its owner announced; refreshing is the owner's
  schedule or the user's explicit action.

## Decision Outcome

Chosen option: the owner's last word, because a value that is fetched on
sight is a poll in another form, and the owner already knows when it
changes.

* The vendor catalog, which changes only with the backend's version, is
  read once in the background when the page starts and cached by its
  version; Settings › Providers shows the providers at once.
* Usage limits show the last snapshot the backend holds, which it already
  announces when it changes. The usage card's refresh button asks the vendor
  again; coming into view does not.
* The backend checks skills' sources on its own schedule, once per source
  however many users installed it, and announces an update; opening
  Settings › Skills checks nothing.
* A device's latency and loss show the last measurement this browser made,
  with when, and are measured again only when the user asks, with a
  Measure button, or when the browser's route to the device changes.
* A Claude Code installation's progress and end are announced by the
  backend that runs it.

### Performance

* Measured: the vendor catalog is 39 KB (212 vendors), read once per
  backend version instead of on each first opening of Providers.
* Estimated: checking each source every five minutes is 288 reads a day per
  source; checking per user instead would be 288 per source per user,
  28.8 million a day for 10,000 users with ten sources each.
* Removed: a vendor request each time usage comes into view, a Git check
  each time Skills opens, twenty probes each time a device's page shows,
  and a read every two seconds during an installation.

### Consequences

* Good, because these views show at once and stay still, and opening them
  costs nothing.
* Bad, because a value can be older than its view, which says when it was
  taken.
