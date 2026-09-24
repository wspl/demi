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
  usage ledger: one row per request       quota snapshot: one per account,
  user, conversation, entry, model,       also filled by free probes:
  token counts                            windows used, reset times, plan
             |                                        |
             v                                        v
  usage totals per user                   the account's usage in settings
```

## Usage ledger

A provider run ends with one response event, which carries the token usage of
the run's final API call, unless the run fails or is cancelled, or is a Claude
Code run that stops at a tool-call batch ([A run](providers.md#a-run)). The
backend meters these events: each one becomes a ledger row holding the user,
the conversation, the provider entry, the model, the input, output, cache-read
and cache-write token counts, and the time
([Storage](../backend/storage.md#control-records)).

Every request a conversation makes passes the same metered runtime: the turns
of its root session and of its subagents, their continuations and automatic
retries, compaction, and title requests
([Conversation titles](../product/product.md#conversation-titles)).

- **Written before the agent continues.** The backend writes the row before
  the response event reaches the agent, so the row exists when the agent moves
  on. A write that fails is logged and the request goes on: inference never
  fails because accounting did.
- **As the vendor reports it.** An HTTP provider's usage comes from the
  vendor's response. Claude Code's usage comes from the CLI's stream-json
  output, relayed over the runner connection from the user's Cloud, so its
  accuracy also depends on the CLI. The CLI reports usage once, on the
  `result` line that ends its turn, and the run's response carries the usage
  of the turn's last API call. When the CLI calls tools during a turn, only
  that last API call therefore has a row
  ([Requests over stream-json](claude-code.md#requests-over-stream-json)). The
  ledger is not an independently verified billing record.
- **Answered requests only.** A request that ends without a response event has
  no row: one that fails, is cancelled, or is refused before it reaches the
  vendor. A request that fails or is cancelled can still have consumed the
  vendor's tokens. A row whose write failed is lost.
- **Totals when read.** The ledger keeps the rows as written and computes
  totals when they are read: one group per provider entry and model, in the
  order each pair was first used, with the number of requests and the sum of
  each token count. Each user sees their own totals; on a shared instance an
  administrator also sees the instance's totals per user
  ([Instance mode](../product/product.md#instance-mode-shared-vs-isolated),
  [Web API](../product/web-api.md#resource-index)).

**Open decision: durable accounting.** As the cases above show, the ledger
does not hold a row for every attempted request or for every API call a
request makes. Billing or strict budgets would need one durable row for each,
with a rule for requests that fail and for writes that fail. Whether Demi
needs that guarantee is not decided, and nothing in the product relies on the
ledger being complete.

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
  [resume it](../product/product.md#recovering-an-unfinished-turn) later.
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
windows    primary     5-hour    34% used    resets 12:30
           secondary   Weekly    12% used    resets Sep 28, 08:00
```

A snapshot holds the account's plan, a label for the account, the time it was
observed, its source, which is a probe or an observation
([Probe and observation](#probe-and-observation)), and its windows. A window
is one limit the vendor meters:

| Field | Meaning |
|---|---|
| `id` | A stable id, such as `primary`, `five_hour` or `monthly` |
| `label` | The name the user knows the window by, such as `5-hour` or `Monthly credits` |
| `usedPercent` | The share used, from 0 to 100, or null when unknown; a reported share above 100 counts as 100, and one below 0 as 0 |
| `used`, `limit`, `unit` | Amounts, when the vendor reports them, in the window's unit, such as `credits`, `requests` or `tokens` |
| `resetsAt` | When the window resets, or null when unknown |
| `severity` | `critical` from 95% used, `warning` from 80%, `normal` below that, and null when the share is unknown; a severity the vendor states is used instead |
| `scope` | What the window applies to when that is narrower than the account, such as one model |

- A window the vendor does not report is absent: Demi never makes up a
  percentage. A vendor field that Demi cannot read is left out, and the rest
  of the snapshot still shows.
- A snapshot holds no token material and no vendor payload.
- A snapshot is information for the user. Inference admission never reads it,
  and neither does the agent.

### Probe and observation

Two paths fill a snapshot:

- A **probe** asks the vendor's usage endpoint about the account. Every probe
  of the three families is free: it reads a usage endpoint and spends no
  inference. That matters most when the limit is reached: a request is then
  refused, and a probe that needed a request could not say why. Demi never
  runs a probe that would cost an inference request
  ([Web API](../product/web-api.md#model-configuration-and-provider-inspection)).
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
| Windows | `primary` and `secondary`, as percentages, labeled by their length: `Weekly` for one week and `Daily` for one day, otherwise the length in the largest unit that divides it evenly, such as `5-hour`, `3-day`, `2-week` or `90-minute`, and `Primary` or `Secondary` when the length is unknown. Both paths use the same labels, so a window keeps its name whichever path saw it last. |
| Plan | `plan_type`, such as `plus`, `pro` or `team`, shown with underscores as spaces and a capital first letter. |
| Account label | The account's label. |
| Units | `reset_at` and the `reset-at` headers are Unix seconds. `limit_window_seconds` is in seconds, and the `window-minutes` headers are in minutes. |

#### Claude Code

| Part | Behavior |
|---|---|
| Probe | `GET https://api.anthropic.com/api/oauth/usage`, with the account's setup token as its bearer token and the OAuth beta header. An answer that is not an object is refused. |
| Observation | The `rate_limits` object on a line of the CLI's stream-json output, at the top level or inside `message`. The backend reads each line as it arrives over the runner connection ([Claude Code](claude-code.md)). |
| Windows | `five_hour` (`5h session`), `seven_day` (`7d all models`), `seven_day_sonnet` (`7d Sonnet`) and `seven_day_opus` (`7d Opus`), from `utilization` or `used_percentage`. Every other entry of `limits` becomes a window named by its `kind` and, for a limit on one model, by that model, with the vendor's `severity` when it states one. The `session` and `weekly_all` kinds are the named windows above, so they are skipped. |
| Plan | None: the account is a setup token, which does not say its plan. |
| Account label | Not reported. |
| Units | `resets_at` is an RFC 3339 time, from the usage endpoint and on the CLI's lines. |

**Open: which of the real CLI's lines carry the windows.** Observation reads
`rate_limits` objects, as the TypeScript provider did, whose recorded examples
carry RFC 3339 times. The CLI's current SDK types
(`@anthropic-ai/claude-agent-sdk`, `sdk.d.ts`) declare no such object on a
stream-json message. They declare a `rate_limit_event` line instead, whose
`rate_limit_info` names one window (`rateLimitType`), its `utilization` and a
numeric `resetsAt`, and they state no unit for either. The `rate_limits` object,
with ISO 8601 `resets_at` times, is the answer to the SDK's `get_usage` control
request. A `rate_limit_event` line therefore changes nothing, and the account's
windows come from probes, until a transcript of the real CLI settles what its
lines carry and in which units
([Acceptance](claude-code.md#acceptance)).

#### Grok Build

| Part | Behavior |
|---|---|
| Probe | `GET /v1/user?include=subscription` and `GET /v1/billing?format=credits`, sent together with the account's session. |
| Observation | The `x-ratelimit-limit-requests`, `x-ratelimit-remaining-requests`, `x-ratelimit-limit-tokens` and `x-ratelimit-remaining-tokens` headers on every chat response, a refusal included. |
| Windows | From billing, one credits window when the plan meters credits: `weekly` (`Weekly credits`) when the current period is weekly, otherwise `monthly` (`Monthly credits`), with the share used (`creditUsagePercent`, otherwise the amount used divided by the limit) and the used and limit amounts. A plan whose billing names none of these has no credits window, since a window with nothing to show is not reported. An on-demand cap above zero adds `on_demand_cap` (`On-demand cap`), which has a limit only. From the headers, `rpm` (`Requests (short window)`) and `tpm` (`Tokens (short window)`), where the amount used is the limit minus what remains. |
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
  ([Web API](../product/web-api.md#model-configuration-and-provider-inspection)).

The browser decides when to ask:

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
