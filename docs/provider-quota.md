# Provider quota (subscription / rate-limit surface)

Final-state design for the unified **quota** API on subscription-style providers
(`codex`, `claude-code`, `grok-build`). Complements multi-credential switching in
[provider-global-credentials.md](./provider-global-credentials.md).

## 1. Problem

Each vendor exposes usage and rate-limit data differently:

| Provider | Active probe | Passive observation |
|---|---|---|
| Codex | `GET …/wham/usage`, the account's usage status | `x-codex-*` on live Responses |
| Claude Code | `GET /api/oauth/usage` | Stream-json `rate_limits` / unified rate-limit headers |
| Grok Build | `/v1/billing?format=credits` + `/v1/user?include=subscription` | Short-window `x-ratelimit-*` on chat |

Products need **one snapshot shape** and two fill paths (probe vs observe), without
vendor-specific UI branches for every header name.

## 2. Goals and non-goals

### Goals

- Shared types: `ProviderQuota`, `ProviderQuotaSnapshot`, `ProviderQuotaWindow`.
- Two fill paths: **active** `probe()` and **passive** `observeResponse()`.
- `latest()`, the last snapshot of **the account the provider was built for**. The snapshot is kept with that
  account by whoever stores it (Demi Next: a column of the account's record; the file store: beside the entry's
  secret), without the vendor's raw payload, so it survives a rebuilt provider and a restart. Another account
  has its own snapshot; switching accounts clears nothing.
- Helper `ensureQuota()` for “cache if fresh, else probe”.
- Wire observation into live inference so quota stays warm without extra probes when possible.

### Non-goals

- Billing UI or payment flows.
- Cross-provider aggregate “total remaining” product metrics (products can compose snapshots).
- Free unlimited probing when a vendor only offers a paid/minimal inference sniff.

## 3. Public contract (`@demicodes/provider`)

### 3.1 Snapshot

```ts
interface ProviderQuotaSnapshot {
  providerId: string
  observedAt: string              // ISO-8601
  source: 'probe' | 'observation' | 'cache'
  plan?: ProviderQuotaPlan | null
  accountLabel?: string | null    // follows active credential when known
  windows: ProviderQuotaWindow[]
  raw?: unknown                   // vendor payload for debugging only
}

interface ProviderQuotaWindow {
  id: string                      // e.g. five_hour, seven_day, primary, monthly
  label?: string
  usedPercent: number | null      // 0–100 when known
  used?: number | null
  limit?: number | null
  unit?: ProviderQuotaWindowUnit  // percent | credits | tokens | …
  resetsAt: string | null         // ISO-8601
  severity?: 'normal' | 'warning' | 'critical' | null
  scope?: { kind: string; label?: string } | null
}
```

### 3.2 `ProviderQuota`

```ts
interface ProviderQuota {
  capability(): ProviderQuotaCapability
  probe(options?: { signal?: AbortSignal; force?: boolean }): Promise<ProviderQuotaSnapshot>
  latest(): ProviderQuotaSnapshot | null
  clearLatest?(): void
  captureObserver(): ProviderQuotaObserver
  observeResponse?(input: {
    headers?: Headers
    status?: number
    body?: unknown   // CLI envelopes without HTTP Response
  }): ProviderQuotaSnapshot | null
}
```

Capability:

```ts
type ProviderQuotaCapability =
  | { mode: 'none' }
  | {
      mode: 'supported'
      canProbe: boolean
      canObserve: boolean
      probeCost?: 'free' | 'minimal_request'
      staleAfterMs?: number
    }
```

- **`free`**: dedicated usage API (Claude, Grok billing).
- **`minimal_request`**: probe burns a tiny inference / streamed request. No shipped provider needs it now.

### 3.3 Helpers

| Helper | Role |
|---|---|
| `createProviderQuota({…})` | Wires probe/observe, holds `latest`, implements `clearLatest` |
| `ensureQuota(quota, { prefer, maxStaleMs, signal })` | Return fresh cache or probe when allowed |
| `clampUsedPercent`, `usedPercentFromRatio`, `severityFromUsedPercent`, `unixSecondsToIso` | Shared numeric/time mapping |

### 3.4 On the `Provider` shell

```ts
interface Provider {
  // …
  quota?: ProviderQuota
}
```

Agent runtime does **not** call quota. Products (web control, REPL, dashboards) do.

## 4. Per-provider mapping

### 4.1 Codex (`@demicodes/provider-codex`)

| Path | Behavior |
|---|---|
| **probe** | Auth from active store → `GET {chatgpt base}/wham/usage` with the same authorization and `ChatGPT-Account-Id` headers a request carries → `plan_type` and `rate_limit.primary_window` / `secondary_window` (`used_percent`, `limit_window_seconds`, `reset_at`). `probeCost: 'free'`. This is the request the open-source Codex CLI makes for its own status (`codex-rs/backend-client`, `get_rate_limits`). |
| **observe** | `x-codex-primary-*` / `x-codex-secondary-*` headers on any live Responses HTTP response (`onHttpResponse` in the provider), a refusal included. |
| **Windows** | `primary` and `secondary`, named by their length (`5-hour`, `Weekly`), with percent + reset. |
| **Plan** | `plan_type` from the usage status (`plus`, `pro`, `team`, …). |

The usage status exists for a ChatGPT sign-in only. An `OPENAI_API_KEY` has no
such account, so its probe fails and its windows come from observation alone.
The probe spends nothing, which matters most exactly when the limit is reached:
a request then is refused, and a probe that needed one could not say why.

### 4.2 Claude Code (`@demicodes/provider-claude-code`)

| Path | Behavior |
|---|---|
| **probe** | Active OAuth token → `GET https://api.anthropic.com/api/oauth/usage` with oauth beta header. `probeCost: 'free'`. |
| **observe** | Stream-json message `rate_limits` via `observeResponse({ body })`, and/or unified Anthropic rate-limit headers when present. |
| **Windows** | e.g. `five_hour`, `seven_day` (and related plan windows when the payload provides them). |

Token resolution follows the same auth path as inference (credential pool active entry, else env / keychain).

### 4.3 Grok Build (`@demicodes/provider-grok-build`)

| Path | Behavior |
|---|---|
| **probe** | Active session → `GET /v1/user?include=subscription` + `GET /v1/billing?format=credits`. `probeCost: 'free'`. |
| **observe** | Short-window `x-ratelimit-*` style headers on chat responses (separate from monthly subscription windows). |
| **Windows** | `weekly` or `monthly` credits from billing, when the plan meters them, plus short request and token windows from headers. A plan whose billing names no used amount, limit or percentage has no credits window: a window with nothing to show is not reported. |

The two sources report different windows, so one never stands for the other. A
snapshot keeps each window until its own source reports it again: a probe
replaces the windows it names and leaves the observed ones, as an observation
leaves the probed ones. Only `clearLatest()` drops a window, since the account
changed.

## 5. Relationship to credentials

- A quota belongs to **one account**: the entry the provider was built for
  ([store contract](provider-global-credentials.md#51-the-store-is-injected)). Probes and observations of that
  provider read and write that account's snapshot only.
- The file store builds its provider for the active pointer, so there `credentials.setActive` still calls
  `quota.clearLatest()`: the same object now stands for another account. A product that builds a provider per
  account never needs it.
- `ProviderQuotaSnapshot.accountLabel` should match the account's label when known.
- `clearLatest()` also invalidates work started before the clear. A pending probe
  rejects with `ProviderQuotaInvalidatedError`; it neither returns the old account's
  snapshot nor replaces the current cache. The caller can issue a new probe for
  the current account.
- Inference adapters call `captureObserver()` before resolving credentials for an
  asynchronous request. They pass response headers or CLI envelopes to the captured
  callback. After invalidation, that callback returns null without changing the
  cache. This covers Codex, Claude Code, and Grok Build, including replies from
  requests that continue after an account switch.
- `observeResponse()` is for immediate observations. An asynchronous integration
  must capture an observer before starting its request; calling the immediate
  method on an old reply cannot identify the reply's account.

Invalidation belongs to the quota instance wired to the credentials API. Products
should reuse that provider instance for switching and querying. An independently
constructed provider or another process is not notified through this in-memory
mechanism.

See [provider-global-credentials.md](./provider-global-credentials.md).

## 6. Product usage

```ts
import { ensureQuota } from '@demicodes/provider'
import { createCodexProvider } from '@demicodes/provider-codex'

const provider = createCodexProvider()

// After a turn, observation may already have filled latest via the transport.
const cached = provider.quota?.latest()

// Or explicitly:
const snap = provider.quota
  ? await ensureQuota(provider.quota, { prefer: 'cache' })
  : null

// Force a network probe (respect probeCost):
const probed = await provider.quota?.probe({ force: true })
```

Guidance:

1. Prefer **observation** during active chat (zero extra cost when headers/body carry windows).
2. Every surface that displays an account's quota checks a **one-minute TTL** when
   that quota region becomes visible, including when no snapshot exists yet. Only an
   account with no completed request or an expired TTL starts a new **free probe**.
   Opening settings, returning to a provider, or scrolling an account back into view
   reuses the result within that minute. The TTL starts when a request completes,
   including a failed request, so switching views cannot repeatedly hit a failing
   vendor. It is shared by provider/account across views in the current app session.
   Snapshot updates and ordinary rerenders do not trigger another probe. The shared
   `web-ui` quota display owns the visibility trigger and its quota refresh cache
   owns the TTL policy; the host supplies requests and retains that cache across views.
3. Keep the last snapshot during refresh and on failure. Automatic requests run
   silently, including on failure, and do not retry while the region remains visible.
   A manual Refresh usage action bypasses the TTL and reports failure. If an automatic
   request is already running, the manual action joins it and receives its outcome.
   Concurrent requests for the same
   provider/account are coalesced; different accounts refresh independently. Closing
   a view lets its request finish and update the account cache; signing out or
   disposing the application cancels pending requests and clears the TTL cache. Adding an account relies on
   its newly visible quota region, rather than issuing a second sign-in probe.
   Providers without a free probe only display observations until an explicit request.
4. Do not poll `probe()` on a timer; use observations and visibility-triggered probes.

## 7. Implementation notes

- Concrete packages own vendor mappers (`mapCodexRateLimitHeaders`, `mapClaudeUsagePayload`, …).
- Secrets never appear on `ProviderQuotaSnapshot` public fields.
- Observation must never throw into the inference stream; provider kits wrap `observeResponse` in try/catch at call sites.
- `createProviderQuota` materializes `providerId`, `observedAt`, and `source` so kits only return plan/windows/account/raw.

## 8. Testing

| Area | Coverage |
|---|---|
| `@demicodes/provider` | `createProviderQuota` cache/observe/`clearLatest`; `ensureQuota` |
| Each kit | Mapper tests and fake transport tests for normal observations and replies arriving after invalidation |
| Credentials | All three kits clear cached quota and invalidate captured observations on setActive; deferred probes cannot overwrite the new account |

## 9. Summary

| Question | Answer |
|---|---|
| One snapshot type for all three? | **Yes** |
| Free probe for all? | **Yes** for the shipped providers; the contract still allows `minimal_request` |
| Primary steady-state path? | **observe** on live inference where possible |
| Agent protocol change? | **None** |
| Tied to multi-cred? | **Yes** — active credential + `clearLatest` on switch |
