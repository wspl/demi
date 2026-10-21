---
status: accepted
---

# The cost shows the last hour

## Context and Problem Statement

A conversation that has run for days shows what it cost since it began.
What the user wants to know while it runs is what it costs now: the last
hour's price is lost in a total of hundreds of dollars.

## Considered Options

* Keep the total alone.
* Let the Cost section switch between the last hour and the whole
  conversation.

## Decision Outcome

Chosen option: a switch, because the total stays one click away and the
last hour answers what the conversation costs now.

```text
  Cost  $412.30                      ┌ Last Hour │ All ┐  ⌄
  ─────────────────────────────────────────────────────
  Input        $3.10      Output       $9.80
  Cache write  $1.20      Cache read   $0.95
  Profiles
  …
  Cache hit rate  97%
  ▁▂▃▅▇▇▆▇█▇
```

* The switch stands at the right of the Cost section's caption, before its
  chevron, once the conversation's first call is more than an hour old;
  before, both would show the same.
* Last Hour counts the calls of the last 60 minutes, and follows the time
  while it shows. It changes the price, the price of each kind of token,
  the profiles' shares and the cache hit rate, and the two figures of the
  closed section. The chart already shows the latest calls and does not
  change.
* All is the default. The choice is the user's, and stays for every
  conversation until changed; the browser keeps it, as it keeps which part of
  the card stands open.
* The server counts the last hour from the ledger's calls, as it counts the
  totals, and the page adds up only what it adds up for the totals. The
  conversation's metadata carries when
  its first call ended, so the switch shows with the card and not after a
  read.
* The labels are chosen as UI text is, by comparing products.

### Performance

* A count of the last hour reads one conversation's calls of that hour,
  through an index by time: some hundreds of rows at most, when the card
  opens, at each new call and once a minute while it shows.

### Consequences

* Good, because a long conversation shows what it costs now.
* Bad, because the section's caption holds one more control.
