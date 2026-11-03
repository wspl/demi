---
status: accepted
---

# A conversation's cost is read by a command

## Problem

Alex asks an agent why the conversation "Refactor the parser" cost so much.
The agent has `demi conversation history timing`, which says where each
turn's time went, and nothing that says where its money went. The composer's
context card shows the conversation's price, cache hit rate and the
profiles' shares, but no command reads them, and nothing anywhere breaks the
price down by turn, by call or by what each call was for. The backend keeps
all of it: one usage ledger row per vendor call, with its agent, turn,
purpose, provider, account, model, token counts, prices, outcome and the
reason it missed the cache. Today the only way to read it is the database.

## Decision

Two commands read the ledger, in the same words as the context card's Cost
section: prices are the equivalent API price, which a subscription's call is
priced at too and which is not a bill.

### `demi conversation cost`

The whole conversation, every agent of it and the calls of no agent, the
counterpart of the context card. `--conversation <id>` reads another of the
user's conversations, with the permission `history` asks for.

```text
$ demi conversation cost
Conversation: c-12
Title: Refactor the parser
Requests: 214
Price ($): 18.4213
Cache hit (%): 91.2
Windows:
  WINDOW  REQUESTS  INPUT   CACHE WRITE  CACHE READ  OUTPUT  PRICE ($)
  all     214       402114  1610322      46210877    611204  18.4213
  day     71        120340  512003       15200110    190876  6.1024
  hour    9         9120    40211        1802114     21003   0.8517
Agents:
  AGENT  PROFILE   REQUESTS  INPUT   CACHE WRITE  CACHE READ  OUTPUT  PRICE ($)  HIT (%)  SHARE (%)
  0      -         150       210300  1100230      38012110    402110  14.2008    92.4     77.1
  1      review    40        102110  310221       6210330     121004  3.1002     88.0     16.8
  2      research  18        89704   199871       1988437     88090   0.9811     90.3     5.3
Models:
  MODEL            ENTRY        REQUESTS  INPUT  …  PRICE ($)
  claude-opus-4-6  Claude Code  150       …         14.2008
Purposes:
  PURPOSE            REQUESTS  INPUT  …  PRICE ($)
  turn               201       …         17.6101
  compaction         4         …         0.6902
  title              1         …         0.0081
  suggested_replies  8         …         0.1129
Media:
  KIND   REQUESTS  ANSWERED  MADE (S)  VENDOR COST  CURRENCY
  image  3         3         -         0.1200       usd
  video  1         1         8.0       0.8000       usd
Account switches: 1
Accounts:
  ENTRY        ACCOUNT           REQUESTS  …  PRICE ($)  FIRST CALL            LAST CALL
  Claude Code  alex@example.com  120       …  10.4012    2026-10-14T09:12:03Z  2026-10-14T11:05:40Z
  Claude Code  team@example.com  80        …  7.6031     2026-10-14T11:06:12Z  2026-10-14T12:40:09Z
Cache misses:
  MISS           CALLS
  cache_expired  3
  model_changed  2
  tools_changed  1
Not answered:
  OUTCOME     REQUESTS  INPUT  …  PRICE ($)
  cancelled   2         …         0.0410
  overloaded  3         …         -
```

- Every value is exact, as every `demi` command prints one: counts in
  tokens, prices in dollars to a hundredth of a cent, instants in UTC.
- Requests are every call to a vendor, a title's, a compaction's and a
  medium's included, as Settings' usage counts them. The card's calls are
  an agent's turn calls alone, and so are the calls a cache miss counts.
- The cache hit rate is over all of the conversation's turn calls; the
  card's is over an agent's latest ones.
- Agent 0 is the main agent; a profile shows `-` for it and for a subagent
  of none.
- The total counts a medium's cost in dollars, as the card does. The Media
  table shows the vendor's own cost in the currency it named, never
  converted: a sum in dollars and one in yuan are two rows.
- An account switch is a turn call of an agent answered by another account
  than its previous turn call, as rotation does. The accounts of an entry
  that another user configures are that user's, as Settings keeps them: they
  are one row with no account, and a switch among them is not counted.
- A table with nothing to show is left out. A price that leaves out calls
  the vendor reported counts for, of a model with no catalog price, says so
  on stderr.

### `demi conversation history cost <range>`

One agent's turns, read as `history timing` reads them: the turns that start
in a range of block indices, fifty to a page, of the caller's transcript,
`--agent`'s or `--conversation`'s.

```text
$ demi conversation history cost 4200..
TURN  STARTED               REQUESTS  CONTEXT FIRST  CONTEXT LAST  INPUT  CACHE WRITE  CACHE READ  OUTPUT  HIT (%)  PRICE ($)  NOTES
4213  2026-10-14T10:02:11Z  12        84102          96220         1210   20114        1003221     9022    97.9     0.4108     -
4260  2026-10-14T10:40:05Z  31        96220          38110         4410   310224       3100112     40021   90.8     1.9017     missed: cache_expired; compacted
4301  2026-10-14T11:20:47Z  3         38110          40002         812    38004        80110       2011    67.4     0.2209     missed: model_changed
```

- CONTEXT FIRST and LAST are the prompts of the turn's first and last turn
  calls: their input, cache write and cache read tokens.
- NOTES name each miss's reason, a compaction in the turn, a call not
  answered and the media the turn made.
- `--calls` prints one row per call instead, under its turn: its number
  among its agent's turn calls, purpose, model, the account that answered
  it, its prompt, the four counts, price, outcome, why it missed the cache
  and when it ended.

### What they are built from

- Both read the control store's `usage_ledger`
  (`crates/backend-database/src/usage.rs`) and nothing else; no row is
  added or changed. They sum its rows with the function that keeps
  each conversation's cost as its rows are written, so their totals are the
  card's. The whole conversation's rows are read by the
  `usage_ledger_conversation` index.
- `history cost` finds a page's turns as `timing` does, then reads the
  agent's rows of the page's turn ids by a new index of the ledger by turn
  id, `usage_ledger_turn`, which a migration adds. A ledger row's turn id
  is its request's, the same id the transcript's blocks keep.
- Each command has `--json`, of the types its tables print. `cost` is a
  leaf of `demi conversation`, and `history cost` of `history`, beside
  `timing`, whose way of finding a page's turns it shares.
- The page shows nothing new.

## Spike

None: every figure is a column of the ledger or a sum the card already
computes.

## Prior art

- Claude Code's `/cost` prints a session's total cost, durations and token
  counts by model.
- [opencode's `stats`](https://learnopencode.com/en/2-daily/05-env-management)
  prints tokens, cost and tool use, by model with `--models`.
- [ccusage](https://npmjs.com/package/ccusage) reports Claude Code's usage
  by session, day and five-hour block, with cache writes and reads apart.

None goes below a session, and none says why a call missed the cache. Demi
breaks the price down by turn and call because its ledger keeps them.

## Alternatives

- **One command under `history`, with the summary as its default.**
  `history` reads one agent's transcript, while the summary is every agent's
  and the calls of none, such as a title's.
- **The figures in `history timing`.** Its rows are about time; a turn's
  price beside its time makes both tables too wide to read.
- **Counting the commands a conversation ran.** It says where the steps
  went, not the money, and is a separate command.

## Open questions

None.
