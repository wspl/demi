# Usage and quota

Demi keeps two records of provider use, and they answer different questions.
The **usage ledger** is Demi's record of what each request consumed, by user,
conversation, provider entry and model. A **quota snapshot** is the vendor's
report of how much of a subscription account's plan is used. Neither one stops
a request; the one limit Demi enforces itself is a per-user **rate limit**.
Provider entries and accounts are defined in [Providers](providers.md).

For example, on a shared instance the master has added a Codex account, and
two users, Ana and Ben, chat with it:

1. Ana sends a message. Before the request leaves, the backend counts it
   against Ana's rate limit: it is her third request this minute, of at most
   120.
2. Codex answers. Its response headers say that the account's 5-hour window is
   34% used, and the backend records that in the account's quota snapshot.
3. The response reports 12,000 input and 800 output tokens. The backend writes
   a ledger row with Ana, her conversation, the Codex entry, the model and the
   token counts, and then the agent continues.
4. Ben's requests consume the same 5-hour window. The snapshot describes the
   account they share; the ledger tells their use apart.

```text
                        one provider request
                                 |
            rate limit: refused above 120 per user per minute
                                 |
                                 v
                          vendor response
                  /                             \
      usage it reports                         quota it reports
             |                                        |
             v                                        v
  usage ledger: one row per call          quota snapshot: one per account,
  user, conversation, agent, account,     also filled by free probes:
  model, token counts, observed quota     windows used, reset times, plan
             |                                        |
             v                                        v
  usage totals per user                   the account's usage in settings
```

## Usage ledger

The ledger has one row for every call Demi makes to a vendor, so the user
can reconcile what a conversation spent with what the vendor counted. For
example, the lead of a conversation answers one message with a turn of 40
calls through the Claude Code CLI, each reading 630,000 tokens from the
cache, while two subagents make 30 calls each. The ledger then holds 100
rows, each naming the agent, the turn, the account and the purpose, and the
row of the call after which the account's 5-hour window was observed at 31%
says so ([Vendor quota](#vendor-quota)).

| Field | Meaning |
|---|---|
| `user_id`, `conversation_id` | Whose call it was |
| `agent` | The number of the session in the conversation's tree, 0 for the root ([Subagents](../agent/subagents.md)); null for a request outside a session, such as a title |
| `turn_id` | The turn the call belongs to: for a command's call, such as an image, the turn whose shell call started the command's job, even when the job outlives it; null outside a turn |
| `purpose` | `turn`, `compaction`, `title`, `image` ([Image generation](../agent/images.md)), `video` ([Video generation](../agent/videos.md)) or `audio` ([Audio generation](../agent/audio.md)): what the call was for |
| `provider_id`, `account_id`, `model_id` | The provider entry, its subscription account (null for an API key) and the model |
| `request_id` | Demi's id of the request, under which the server log holds its course |
| `vendor_id` | The vendor's id of the call, such as a Messages API message id, when the vendor gives one |
| `input_tokens`, `output_tokens`, `cache_read_tokens`, `cache_write_tokens`, `cache_write_1h_tokens` | The vendor's counts for this call alone ([Usage](providers.md#usage)), each null when the vendor reported none, as for a call cancelled before it answered; `cache_write_1h_tokens` is the part of the cache write kept for an hour, which a vendor prices apart, 0 where the vendor has no such tier |
| `media_seconds` | The length of the video or audio the call made, in seconds, as the file states it; null for any other call or when nothing was made |
| `cost`, `cost_currency` | What the vendor says the call cost, in its documented unit converted to the currency it names (`usd` or `cny`, never converted between them), as xAI's `cost_in_usd_ticks` are ten-billionths of a dollar; null when the vendor reports no cost or one in an undocumented unit |
| `api_price_input`, `api_price_cache_write`, `api_price_cache_read`, `api_price_output` | What each kind of the call's tokens would cost on the vendor's API, in dollars, fixed when the row is written ([A conversation's cost](#a-conversations-cost)); null when the model has no price for them |
| `system_hash`, `tools_hash` | Hashes of the system prompt and of the tool definitions as the request sent them, so a cache miss can be told apart from a prompt that changed; null for a request outside a session |
| `outcome` | `answered`; the failure code of a call the vendor refused ([Failures and recovery](../agent/failures-and-recovery.md)); `failed` for a failure with no code; `cancelled` |
| `quota` | The windows of the account's quota that this call's own response reported, each as the snapshot names a window, its kind, period and scope, with `usedPercent` and `resetsAt`; null when the call reported none. Windows the snapshot holds from other calls are not repeated here. A window the vendor reports after the call's end and before the next call starts, as Claude Code's `rate_limit_event` follows its `message_stop`, is this call's and is added to its row when it arrives |
| `started_at`, `ended_at` | When the call was sent and when it ended |

- **Every call.** Every call a conversation makes passes the same metered
  runtime: the turns of its root session and of its subagents, their
  continuations and automatic retries, compaction, and title requests
  ([Conversation titles](../product/conversations.md#conversation-titles)). A run
  of several calls, such as a Claude Code run in which the CLI calls tools,
  writes one row per call: the CLI's stream gives each call's
  `message_start` and `message_delta`, read as the run's response usage is
  ([Requests over stream-json](claude-code.md#requests-over-stream-json)),
  never the `result` line's sum.
- **The log meets the ledger.** Each call's lines in the server log carry
  its `request_id`, its agent and its account, so a row found in the ledger
  finds its course in the log, and a question the ledger answers, such as
  which account an agent used after a compaction, the log answers too while
  it keeps the lines.
- **Written as each call ends.** The backend writes a call's row when the
  call ends, and the row of a run's last call before the run's response
  reaches the agent. A write that fails is logged and the request goes on:
  inference never fails because accounting did, and the row is lost.
- **As the vendor reports it.** The counts are the vendor's: an HTTP
  provider's from its response, Claude Code's from the CLI's stream relayed
  from the user's Cloud, so their accuracy also depends on the CLI. The
  ledger is a record for reconciliation, not a bill.
- **A call cancelled before the vendor answered** has a row with the outcome
  `cancelled` and no counts; the vendor may still have counted its input.
- **Totals when read.** The ledger keeps the rows as written and computes
  totals when they are read: one group per provider entry and model, in the
  order each pair was first used, with the number of calls and the sum of
  each token count. Each user sees their own totals; on a shared instance an
  administrator also sees the instance's totals per user
  ([Instance mode](../product/users.md#instance-mode-shared-vs-isolated),
  [Web API](../product/web-api.md#resource-index)).
- **Kept** as long as the account that made the calls; deleting a
  conversation keeps its rows ([Storage](../backend/storage.md#control-records)).

## A conversation's cost

The context card shows what a conversation has cost and how well the
vendor's prompt cache served it, under the context window's part and above
the usage limits ([What fills the context](../agent/compaction.md#what-fills-the-context)).
It answers two questions a user asks of a long conversation: what would
this have cost on the API, and is something breaking the cache. For
example, a conversation whose main agent made 180 calls and whose
subagents made 40 more:

```text
Cost
$41.20                       97%
Equivalent API price         Cache hit rate
```

Opened with the context window's list, the same `⌄`:

```text
Cost
Equivalent API price                     $41.20
Uncached input            53.8K           $0.07
Cache write                3.2M          $12.64
Cache read                51.9M          $22.96
Output tokens              308K           $5.53

Profiles
Main conversation                        $36.37
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
research                                  $2.64
━━━━━
review                                    $1.32
━━

Cache hit rate  97%          [Requests | Time]
▁▇█▇█▇█▇▆█▇█│▇█▇█▇█▁▇█▇█▇█│▇▇█▇█▁▇█▇█▇█▇█▇█▇█▇█
#1        50        100        150          180
```

- **Closed**, the section is the caption *Cost* and two figures side by
  side: the equivalent API price at the text's left edge and the cache hit
  rate at the middle, each its value in the primary color over its label in
  small secondary text.
- **Opened**, the figures become rows as the context window's are: the
  price, then the four token kinds, each its tokens and its price in the
  secondary color, the price row's total in the primary color.
- **Profiles** is a sub-caption, in the caption's style, with the price
  of the subagents of each profile the conversation's workflow offers now
  ([Workflows of a project](../agent/subagents.md#workflows-of-a-project)):
  a project workflow's profiles, or under Global Workflow the user's enabled
  profiles, none while the user has subagents turned off. The main conversation comes first for scale, then each such
  profile whose subagents made priced calls, each a row with its price and
  a thin bar under it whose length is its share of the largest. A subagent
  started with no profile, or with one the current workflow does not offer,
  has no row. With No Subagents, or when no profile of the current workflow
  ran a subagent, the part is left out.
- **Cache hit rate** is a sub-caption too, its value after it in the
  primary color, with the switch *Requests | Time* at the row's end. The
  chart under it plots the main conversation's turn calls, each at its
  call's hit rate, by its number or by its start time: no axis labels on
  the side, faint lines at 0, 50 and 100%, the numbers or times under it.
  A cache miss is a red dot at its call, a compaction a grey dashed line
  between the calls it separates. Resting the pointer on the chart, or a
  tap on a phone, shows the nearest call's tip: its number, its time, its
  hit rate and its price, and for a miss its cause in red and the tokens it
  wrote again; the first call reads *First request* and the first after a
  compaction *After compaction*, in the secondary color, saying why their
  low rate is no miss. The time is the clock time and how long ago,
  *13:17 · 1h 27m ago*, with the date when it is not today. The card keeps
  the Requests or Time choice in the browser, as it keeps the agents'. The tip stands beside the point where it fits, otherwise
  above it, and never past the card's edges. The chart, and the switch
  with it, shows only once the main conversation has made 4 calls: with
  fewer a line shows nothing a user can read, and the rate alone says it.
- **A read that fails says so.** When the page cannot read the cost, the
  section shows its caption and, in the secondary color, *Couldn't load
  the cost* with the request's id and a Retry button, as the usage limits
  show a failed refresh, never an empty section a user would take for no
  calls.
- **Nothing to show is no part.** A conversation with no calls yet has no
  Cost section; a figure that cannot be computed, a price when no call has
  one, is left out with its label.

### How the figures are counted

The figures come from the conversation's [ledger rows](#usage-ledger), all
of its calls: every agent, compactions and titles included, cancelled
calls with no counts adding nothing.

| Figure | Counted as |
|---|---|
| Uncached input, Cache write, Cache read, Output tokens | The sums of `input_tokens`, `cache_write_tokens`, `cache_read_tokens` and `output_tokens` |
| A token kind's price | The sum of that kind's `api_price_…` over the calls that have one |
| Equivalent API price | The sum of each call's price: its four parts, or, for a call whose tokens have no price, such as an image's, the vendor's own cost in dollars. The kinds' prices then add up to less than it by those calls |
| Cache hit rate | Over the main conversation's turn calls: cache read ÷ (uncached input + cache write + cache read), which is the share of the prompt the vendor read from its cache. Subagents are left out: each has its own cache, and a short-lived one starts cold by nature |

A call's **price** is fixed when its row is written, at the catalog's
prices at that moment, so a later change of price never rewrites history
and a price the catalog learns mid-conversation applies from the next
call. It is the call's tokens at the model's
catalog prices ([Models](models.md)), one part per kind: input, cache read
and cache write at their rates, the part of the write kept for an hour at
twice the input rate, as the Messages API prices it, and output at its
rate. A model without a price for a kind the call used leaves the call's
parts null; such a call is priced at the vendor's own cost when it reports
one in dollars ([Usage ledger](#usage-ledger)), and otherwise has no price.
The catalog's price comes first even when the vendor reports a cost, so the
kinds always add up to the call's price. A subscription's call, such as Claude Code's or Codex's, is priced as
its model is on the vendor's API: that is the *equivalent*. A
subscription's catalog has no prices, so its model's are those of the same
model on the vendor's API, Anthropic's for Claude Code, OpenAI's for Codex
and xAI's for Grok Build, from the same price source the API entries' catalogs
use; a model the API does not sell has no price.

**A cache miss** is a turn call of the main conversation that wrote again
what an earlier call had cached: uncached input plus cache write over 5% of
the call's prompt and at least 2,000 tokens, as Claude Code's own cache
diagnosis counts one. The first call of the conversation, and the first
after a compaction, which starts a new prompt, are expected to write and are
never misses. A miss's cause is read from its row and the previous turn
call's, the first that applies:

| Cause | When |
|---|---|
| Model changed | The two rows' models differ |
| Tools changed | Their `tools_hash` differ |
| System prompt changed | Their `system_hash` differ, as when an instructions file changed |
| Cache expired | The time between the previous call's end and this call's start is longer than the vendor keeps a prompt: an hour after a write kept for an hour, five minutes otherwise, the write being the latest one of the same model |
| Not cached by the provider | None of the above: the vendor dropped or never stored the prompt |

### Reading it

`GET /api/conversations/:id/cost` answers the section's data, computed from
the ledger when read ([Conversation history](../product/web-api-conversations.md#subagents-and-commands)):

```json
{
  "totals": { "inputTokens": 53800, "cacheWriteTokens": 3200000,
              "cacheReadTokens": 51900000, "outputTokens": 308000,
              "price": { "total": 41.2, "input": 0.07, "cacheWrite": 12.64,
                         "cacheRead": 22.96, "output": 5.53 } },
  "agents": [ { "agent": 0, "profile": null, "price": 36.37 } ],
  "calls": [ { "startedAt": "…", "inputTokens": 1200, "cacheWriteTokens": 900,
               "cacheReadTokens": 104000, "price": 0.17,
               "afterCompaction": false, "miss": null } ]
}
```

`agents` lists every agent with a priced call; `calls` is the main
conversation's turn calls in order, `miss` the cause when the call is one.
Each price is null when no call it sums has one. A call is
`afterCompaction` when the main conversation answered a compaction between
it and the previous turn call. The page reads it when the card opens and
keeps it while the card is open; the backend announces each new row of the
conversation with a `cost` frame carrying what the row adds, to the totals,
to its agent and, for a main conversation's turn call, the call itself,
which the page adds, so an open card follows the conversation without
asking again. The answer and each frame carry the number of the
conversation's ledger rows they include, so the page applies a frame only
when it follows what it holds: a frame it already holds, as one that
crossed the read, is dropped, and one past a gap makes it read the cost again, because it knows rows
are missing rather than hoping some are. A connection that attaches again
after it was down receives the conversation's row count with the state an
attach brings, so a page that missed the last rows, with no frame after
them to show the gap, reads the cost again too. Prices tiered by a
request's length, as some models charge more past 200K tokens, are not
modelled: a call is priced at the model's base rates.

## Rate limit

Each user's conversations may start at most 120 provider requests in any 60
seconds. Every request that the [usage ledger](#usage-ledger) meters counts,
whatever provider entry or model it uses. For example, a user whose three
subagents loop on a failing step reaches 120 requests within 40 seconds.
The 121st request is refused, and requests are admitted again as the earliest
ones leave the 60-second window.

- **Checked after the entry.** The backend first resolves the provider entry. A
  missing, deleted or inaccessible entry, or a subscription entry without an
  account, refuses the request without taking a slot
  ([Inference admission](providers.md#inference-admission-and-runtime-ownership)).
- **A refusal fails the attempt.** A request over the limit never reaches the
  vendor and does not count. The attempt fails with the code `rate_limited`,
  which is distinct from a vendor's `rate_limit`: the agent does not retry it
  by itself ([Retries](../agent/failures-and-recovery.md#retries)), so the turn
  stops with the failure, and the user can
  [resume it](../product/turns.md#recovering-an-unfinished-turn) later.
- **A ceiling, not a budget.** The limit is generous: it stops runaway loops
  and abuse, and it does not price use. Demi enforces no token or spending
  budget per user, and a quota snapshot is information, not a limit.
- **Kept per process.** The window is held in memory with the user's other
  state on the [user's shard](../architecture/concurrency.md#the-user-shard),
  and a restart empties it. A multi-worker deployment places all of a user's
  work on one worker
  ([Deployment](../backend/backend.md#deployment-and-user-ownership)), so the
  window still counts every request of that user. It is not a distributed
  limiter, and neither is the
  [login lockout](../backend/backend.md#authentication-and-ownership), which
  each process also keeps in memory.

## Vendor quota

A subscription vendor reports how much of an account's plan is used. Demi
keeps the latest report of each account as the account's **quota snapshot**.
Only the subscription families have one: `codex`, `claude-code` and
`grok-build`. API-key entries have no quota.

### What a snapshot says

For example, the snapshot of a Codex account after a response:

```text
plan       Pro
observed   2026-09-23T10:04:12Z, by an observation
windows    primary     limit, 5 h  34% used    resets 12:30
           secondary   Weekly    12% used    resets Sep 28, 08:00
```

A snapshot holds the account's plan, a label for the account, the time it was
observed, its source, which is a probe or an observation
([Probe and observation](#probe-and-observation)), and its windows. A window
is one limit the vendor meters:

| Field | Meaning |
|---|---|
| `id` | A stable id, such as `primary`, `five_hour` or `monthly` |
| `kind` | What it meters: `limit`, a share of use; `credits`; `requests` or `tokens`, a rate; `cap`, a ceiling with no period. The page names the window from it ([Window names](#window-names)); a snapshot holds no name |
| `usedPercent` | The share used, from 0 to 100, or null when unknown; a reported share above 100 counts as 100, and one below 0 as 0 |
| `used`, `limit`, `unit` | Amounts, when the vendor reports them, in the window's unit, such as `credits`, `requests` or `tokens` |
| `resetsAt` | When the window resets, or null when unknown |
| `periodSeconds` | How long one period of the window lasts, as the vendor states it (Codex's `limit_window_seconds`, Claude's five-hour and seven-day windows, Grok's weekly credits or billing period), or null when unknown |
| `severity` | `critical` from 95% used, `warning` from 80%, `normal` below that, and null when the share is unknown; a severity the vendor states is used instead |
| `scope` | What the window applies to when that is narrower than the account, such as one model |

- A window the vendor does not report is absent: Demi never makes up a
  percentage. A vendor field that Demi cannot read is left out, and the rest
  of the snapshot still shows.
- A snapshot holds no token material and no vendor payload.
- A snapshot is information for the user and for choosing an account under
  [Account rotation](providers.md#account-rotation). Inference admission
  never reads it, and the model never sees it.

Every surface that shows a window says its use and its reset the same way, on
one line over its bar, the window's name, then its reset and its use at the
line's end, *5-hour limit · · · Resets in 4h 42m · 4% used*, the reset
giving way first where the line is short: a bar of `usedPercent`, *18% used*, and when it resets, as specifically as the
user would say it, counted from now in the user's time zone:

| When it resets | Wording |
| --- | --- |
| Passed, before the next snapshot | *Resets now* |
| Within a minute | *Resets in <1m* |
| Within the hour | *Resets in 42m* |
| Within 24 hours | *Resets in 3h 42m*, *Resets in 3h* on the hour |
| 24 hours or more, less than 7 days | *Resets Sat 4:00 PM*: 7 days ahead would name today's weekday, so the weekday stops at 6 |
| 7 days or more | *Resets Oct 24*, the day alone; *Resets Jan 3, 2027* when it is not this year |
| Unknown | No reset line |

The wording follows the clock while it shows. One web-ui row and one
formatter say it on every surface, Settings and the context card alike, so
the two never differ.

#### Window names

The page names a window in the user's language, from its kind, its period
and its scope, the same way for every vendor, so Codex's and Claude's
five-hour windows read alike, as ChatGPT and Claude name them in their own
settings, by their length and *limit*:

| Window | English | 简体中文 |
| --- | --- | --- |
| A limit of a period | *5-hour limit*, *Daily limit*, *Weekly limit*, *3-day limit* | *5 小时限额*、*每日限额*、*每周限额*、*3 天限额* |
| A limit on one model | *Weekly Opus limit* | *每周 Opus 限额* |
| Credits of a period | *Weekly credits*, *Monthly credits* | *每周额度*、*每月额度* |
| A rate | *Request rate*, *Token rate* | *请求速率*、*Token 速率* |
| A cap | *On-demand cap* | *按需用量上限* |
| A limit whose period is unknown | *Usage limit*, *Opus limit* | *用量限额*、*Opus 限额* |

A period reads *Daily* for one day, *Weekly* for one week, *Monthly* for 28
to 31 days, and otherwise its length in the largest unit that divides it
evenly; a credits window whose period is unknown is *Credits*. Inside a
sentence, such as the account switch's line, an English name is lowercase
but for a proper name: *reached its weekly Opus limit*. The other languages
say what the English says
([Localization](../product/localization.md)).

Settings show every account's
windows, and the context indicator's card shows, under the context's rows,
the windows of the account the conversation's model runs on, under a line
naming the account by the label Settings shows for it, *Usage limits ·
alex@example.com*, so a user with two accounts of one provider sees which one
the limits are; the line is *Usage limits* alone for an account without a
label, and a long label gives way at its end. The card does not name the
plan, which Settings shows beside the account. A model on an API key has no
such part.

### Probe and observation

Two paths fill a snapshot:

- A **probe** asks the vendor's usage endpoint about the account. Every probe
  of the three families is free: it reads a usage endpoint and spends no
  inference. That matters most when the limit is reached: a request is then
  refused, and a probe that needed a request could not say why. Demi never
  runs a probe that would cost an inference request
  ([Web API](../product/web-api-settings.md#model-configuration-and-provider-inspection)).
- An **observation** reads the quota a vendor reports alongside a live
  inference response, in the response's headers or in the lines of the Claude
  Code CLI's output. It costs nothing extra, so it keeps snapshots current
  while users chat. An observation never fails a request: a response it cannot
  read is ignored.

The two paths can report different windows. A probe sets the plan and the
account label and replaces the windows it names; an observation replaces the
windows it names and keeps the plan and the label. A window the update does
not name stays, and a new window is added after the others. Each window
therefore shows what its own source said last: for example, Grok Build's
monthly credits come from probes and its request window from observations, and
neither replaces the other. Each update reads, merges and writes the account's
snapshot as one step, so a probe and an observation that arrive together both
land.

### One snapshot per account

A provider is built for one account
([An account is the unit](providers.md#an-account-is-the-unit)), and so is its
quota: the probes and observations of a provider read and write that account's
snapshot only. A request that outlives a change of the active account still
credits the account that made it, so changing the active account needs no
invalidation.

- Selecting another account changes which account inference uses. It clears
  nothing: the account left behind keeps its snapshot until its next probe or
  observation.
- Removing an account removes its snapshot.
- The snapshot is stored in the account's record, without the vendor's payload
  ([Storage](../backend/storage.md#control-records)), so it outlives a rebuilt
  provider and a restart. Each backend process holds the snapshots it uses in
  memory and writes every update through to the record. A failed write is
  logged; it costs at most a later probe. A stored snapshot that cannot be read
  is an error, never an empty snapshot.
- With several workers, each worker holds its own copy. A snapshot that
  another worker stored appears after this worker's next probe of the account
  or its next restart.
- On a shared instance every user infers with the master's accounts, so the
  vendor's windows are consumed by all of them together. The snapshot
  describes the account, not a user; per-user use is the
  [usage ledger's](#usage-ledger). Only the master sees accounts, plans and
  usage ([Scope](providers.md#scope)).

### Per vendor

A reset time is read in the unit that its vendor field declares, as the
following tables list. A value that does not read in that unit is an unknown
reset time.

#### Codex

| Part | Behavior |
|---|---|
| Probe | `GET /wham/usage` on the ChatGPT backend, with the authorization and `ChatGPT-Account-Id` headers a request carries. It reads `plan_type` and the `rate_limit` object's `primary_window` and `secondary_window`: `used_percent`, `limit_window_seconds` and `reset_at`. This is the request the Codex CLI makes for its own status. |
| Observation | The `x-codex-primary-*` and `x-codex-secondary-*` headers, `used-percent`, `window-minutes` and `reset-at`, on every response of Codex's service, the answer to a WebSocket handshake and a refusal included. |
| Windows | `primary` and `secondary`, as percentages, each a `limit` with its length as its period, which may be unknown. |
| Plan | `plan_type`, such as `plus`, `pro` or `team`, shown with underscores as spaces and a capital first letter. |
| Account label | The account's label. |
| Units | `reset_at` and the `reset-at` headers are Unix seconds. `limit_window_seconds` is in seconds, and the `window-minutes` headers are in minutes. |

#### Claude Code

| Part | Behavior |
|---|---|
| Probe | The CLI's SDK control request `get_usage` (`skip_behaviors`), in a process of the account's directory that sends no message ([Accounts and sign-in](claude-code.md#accounts-and-sign-in)): the CLI renews the sign-in when due and answers `rate_limits`, the vendor's usage object, and `subscription_type`. A `rate_limits` that is not an object fails the probe. |
| Observation | The `rate_limit_event` line of the CLI's stream-json output, which the CLI prints when the vendor's `anthropic-ratelimit-unified-*` headers change. Its `rate_limit_info.unifiedWindows` gives each window's `utilization` and `resetsAt`; the window `rateLimitType` names as binding, with the top-level `utilization` and `resetsAt`, counts too when `unifiedWindows` does not hold it. The backend reads each line as it arrives over the runner connection ([Claude Code](claude-code.md)). |
| Windows | `five_hour`, `seven_day`, `seven_day_sonnet` and `seven_day_opus` (scope Sonnet, Opus), each a `limit`, from `utilization`. A probe also reports every other entry of `limits` as a `limit`, with the vendor's `severity` when it states one, by its period and its scope, never by the vendor's identifier: a `weekly_*` kind has a week, a session kind five hours, and a limit on one model has the model as its scope, so `weekly_scoped` on Fable reads *Weekly Fable limit*; a kind whose period Demi cannot tell has none. The `session` and `weekly_all` kinds are the named windows above, so they are skipped. An observed window Demi does not name, such as `seven_day_overage_included`, is left out. |
| Plan | `subscription_type`, such as `max` or `pro`, shown with a capital first letter. |
| Account label | Not reported. |
| Units | The usage object states `utilization` as a percentage and `resets_at` as an RFC 3339 time. A `rate_limit_event` line passes the headers through: `utilization` is a fraction of one and `resetsAt` is Unix seconds (Claude Code 2.1.286, against a scripted vendor that sent the headers). |

#### Grok Build

| Part | Behavior |
|---|---|
| Probe | `GET /v1/user?include=subscription` and `GET /v1/billing?format=credits`, sent together with the account's session. |
| Observation | The `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests`, `x-ratelimit-limit-tokens` and `x-ratelimit-remaining-tokens` headers on every chat response, a refusal included. |
| Windows | From billing, one credits window when the plan meters credits: `weekly` when the current period is weekly, otherwise `monthly`, of kind `credits`, with the share used (`creditUsagePercent`, otherwise the amount used divided by the limit) and the used and limit amounts. A plan whose billing names none of these has no credits window, since a window with nothing to show is not reported. An on-demand cap above zero adds `on_demand_cap`, a `cap`, which has a limit only. From the headers, `rpm` and `tpm`, of kind `requests` and `tokens`, where the amount used is the limit minus what remains. |
| Plan | `subscriptionTier` from the user endpoint. |
| Account label | The account's email address. |
| Units | A credits window resets at `currentPeriod.end`, otherwise at `billingPeriodEnd`, both RFC 3339 times. The header windows have no reset time. |

### Refreshing in the product

The backend never probes by itself and never polls. Observations keep
snapshots current while users chat, and a probe runs only when the product asks
for one:

- Reading a provider's status returns the kept snapshot of each account; it
  never probes and never runs inference.
- `POST /api/providers/:id/quota` probes one account: the one it names, or the
  active one
  ([Web API](../product/web-api-settings.md#model-configuration-and-provider-inspection)).

The web app decides when to ask:

- Every surface that shows an account's quota asks for a probe when that quota
  region becomes visible, including when the account has no snapshot yet,
  unless the account's last request completed less than a minute ago. The
  minute starts when a request completes, a failed one included, so switching
  views cannot hit a failing vendor again and again. It is kept per provider
  and account, across views, for as long as the application runs. Opening
  settings, returning to a provider, or scrolling an account back into view
  within that minute reuses the result. A snapshot update or an ordinary
  rerender asks for nothing.
- The last snapshot stays on screen while a request runs and after one fails.
  An automatic request runs silently, including when it fails, and it is not
  repeated while the region stays visible.
- A manual **Refresh usage** action ignores the minute and reports a failure.
  Settings offers it on each account, and the context card at the end of its
  *Usage limits* line, as an icon button with the tip *Refresh usage*: the
  icon turns while the request runs, and the windows stay as they were until
  the answer replaces them. A failure is a fact of the account, not of the
  surface that asked: it shows where that account's windows show, under the
  card's header and under the account in Settings, in the secondary color
  with the vendor's words, wrapping when they are long, until the account's
  next answer. No toast reports it, since the line already stands where the
  user looked.
  If an automatic request for the account is running, the manual action joins
  it and receives its outcome.
- Concurrent requests for one provider and account share one request;
  different accounts refresh independently.
- Closing a view lets its request finish and update the account's cached
  result. Signing out, or closing the application, cancels pending requests
  and forgets every minute.
- Adding an account needs no probe of its own: the new account's quota region
  becomes visible and asks.
- The quota display in `web-ui` starts a request when its region becomes
  visible, and its refresh cache applies the minute. The application around it,
  `web` or the gallery, supplies the requests and keeps that cache across
  views.
