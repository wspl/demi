---
status: proposed
---

# The cost shows the last hour, the last day or all of it

Supersedes, in part: 0041 (the cost shows the last hour), where the switch
holds two ranges, Last Hour and All.

## Problem

A conversation has run for three days. Its total is $412, its last hour
$3. What it cost today, the work of this morning and of last night, is in
neither: the user cannot see it.

## Decision

The switch holds three ranges, in order of their span: the last hour, the
last day, and all.

```text
  Cost  $38.70                    ┌ 1H │ 1D │ All ┐  ⌄
  ─────────────────────────────────────────────────
  Input        $9.10      Output       $21.40
  …
```

* **The last day** counts the calls of the last 24 hours, and follows the
  time while it shows, as the last hour counts its 60 minutes. It changes
  what the last hour changes, and the chart does not change.
* **When each range shows:** a range shows once the conversation's first
  call is older than its span, since before that it would show what All
  shows. A conversation of three hours offers 1H and All; one of three
  days offers all three. 0041's rule for the last hour, applied to the
  day.
* **The choice** stays the user's for every conversation, as 0041 has it.
  A conversation that does not offer the chosen range shows All.
* **The labels** are 1H, 1D and All, as Stocks and Coinbase write them,
  which the mini switch holds; every other language says what the English says.
  Last Hour, Last 24 Hours and All read without stopping but take twice
  the width.

The design to build from:

* `GET /conversations/:id/cost/recent?window=hour|day` replaces
  `GET /conversations/:id/cost/last-hour`, with the same answer under new
  names:

  ```rust
  pub enum CostWindow { Hour, Day }        // "hour", "day"
  pub struct RecentCost { pub agents: Vec<RecentAgent> }
  pub struct RecentAgent { pub agent: Option<u64>, pub cost: AgentCost,
                           pub rate: Option<f64> }
  ```

* The backend counts a window from the ledger's calls through the index by
  time it counts the last hour through; nothing new is stored.
* The page's `CostRange` becomes `'hour' | 'day' | 'all'`, and it reads
  only the window the card shows, at the same times as today. The choice
  the browser kept as `lastHour` reads as `hour`.
* `firstCallAt` decides both offers; the metadata does not change.
* An older page still open reads the old route, which is gone: its cost
  read fails, as its toast says, until it reloads.

### Performance

* A count of the last day reads one conversation's calls of that day
  through the index: some thousands of rows for a busy day, when the card
  opens, at each new call and once a minute while it shows.

## Spike

None.

## Prior art

* macOS's Stocks app: its range selector reads 1D, 1W, 1M, … ALL, in order
  of their span, and the range chosen applies to every ticker symbol
  ([Change the chart display](https://support.apple.com/guide/stocks/change-the-chart-display-stc4cfc704df/mac)).
  Demi's choice applies to every conversation the same way.
* Coinbase's price chart: 1H, 1D, 1W, 1M, 1Y, All
  ([an asset's page](https://www.coinbase.com/price/bitcoin)), the hour
  among them.

## Alternatives

* **The calendar day, since midnight.** A conversation that works across
  midnight shows little in the morning, and the hour is a rolling window
  already.
* **Always three ranges.** A conversation younger than a day shows the
  same under 1D and All.

## Open questions

None.
