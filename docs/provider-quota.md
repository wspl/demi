# Provider quota (subscription / rate-limit surface)

Final-state design for the unified **quota** API on subscription-style providers
(`codex`, `claude-code`, `grok-build`). Complements multi-credential switching in
[provider-global-credentials.md](./provider-global-credentials.md).

## 1. Problem

Each vendor exposes usage and rate-limit data differently:

| Provider | Active probe | Passive observation |
|---|---|---|
| Codex | Minimal Responses request (headers only) | `x-codex-*` on live Responses |
| Claude Code | `GET /api/oauth/usage` | Stream-json `rate_limits` / unified rate-limit headers |
| Grok Build | `/v1/billing` + `/v1/user?include=subscription` | Short-window `x-ratelimit-*` on chat |

Products need **one snapshot shape** and two fill paths (probe vs observe), without
vendor-specific UI branches for every header name.

## 2. Goals and non-goals

### Goals

- Shared types: `ProviderQuota`, `ProviderQuotaSnapshot`, `ProviderQuotaWindow`.
- Two fill paths: **active** `probe()` and **passive** `observeResponse()`.
- In-memory `latest()` cache with optional `clearLatest()` (required after credential switch).
- Helper `ensureQuota()` for “cache if fresh, else probe”.
- Wire observation into live inference so quota stays warm without extra probes when possible.

### Non-goals

- Billing UI or payment flows.
- Cross-provider aggregate “total remaining” product metrics (products can compose snapshots).
- Free unlimited probing when the vendor only offers a paid/minimal inference sniff (Codex).

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
- **`minimal_request`**: probe burns a tiny inference / streamed request (Codex).

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
| **probe** | Auth from active store → minimal streamed Responses → parse `x-codex-primary-*` / `x-codex-secondary-*` → cancel body. `probeCost: 'minimal_request'`. |
| **observe** | Same headers on any live Responses HTTP response (`onHttpResponse` in the provider). |
| **Windows** | Typically `primary` and `secondary` with percent + reset. |

There is no free Codex “usage only” HTTP API in this stack; passive observe on SSE is the preferred steady-state path.

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
| **probe** | Active session → `GET /v1/user?include=subscription` + `GET /v1/billing`. `probeCost: 'free'`. |
| **observe** | Short-window `x-ratelimit-*` style headers on chat responses (separate from monthly subscription windows). |
| **Windows** | e.g. `monthly` (billing) plus optional short RPM/TPM-style windows from headers. |

## 5. Relationship to credentials

- Quota always reflects the **global active** credential for that provider.
- On `credentials.setActive`, implementations **must** call `quota.clearLatest()` so UI does not show the previous account’s windows.
- `ProviderQuotaSnapshot.accountLabel` should match the active account label when known.

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

// Force a network probe (respect probeCost — Codex is not free):
const probed = await provider.quota?.probe({ force: true })
```

Guidance:

1. Prefer **observation** during active chat (zero extra cost when headers/body carry windows).
2. Call **probe** for dashboard open / refresh, or when `latest()` is null/stale.
3. For Codex, avoid polling `probe()` on a tight timer; use observe + sparse probe.

## 7. Implementation notes

- Concrete packages own vendor mappers (`mapCodexRateLimitHeaders`, `mapClaudeUsagePayload`, …).
- Secrets never appear on `ProviderQuotaSnapshot` public fields.
- Observation must never throw into the inference stream; provider kits wrap `observeResponse` in try/catch at call sites.
- `createProviderQuota` materializes `providerId`, `observedAt`, and `source` so kits only return plan/windows/account/raw.

## 8. Testing

| Area | Coverage |
|---|---|
| `@demicodes/provider` | `createProviderQuota` cache/observe/`clearLatest`; `ensureQuota` |
| Each kit | Mapper unit tests; optional live probe gated by env |
| Credentials | setActive clears quota cache (kit-level) |

## 9. Summary

| Question | Answer |
|---|---|
| One snapshot type for all three? | **Yes** |
| Free probe for all? | **No** — Codex is `minimal_request` |
| Primary steady-state path? | **observe** on live inference where possible |
| Agent protocol change? | **None** |
| Tied to multi-cred? | **Yes** — active credential + `clearLatest` on switch |
