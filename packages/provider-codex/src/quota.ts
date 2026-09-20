import { randomUUID } from 'node:crypto'
import {
  clampUsedPercent,
  numberHeader,
  createProviderQuota,
  severityFromUsedPercent,
  unixSecondsToIso,
  type ProviderQuota,
  type ProviderQuotaProbeResult,
  type ProviderQuotaSnapshots,
  type ProviderQuotaWindow,
} from '@demicodes/provider'
import { FileCodexAuthStore, type CodexAuthStore } from './auth'
import { z } from 'zod'
import { buildCodexHeaders } from './provider'

export interface CodexQuotaOptions {
  providerId?: string
  codexHome?: string
  baseUrl?: string
  authStore?: CodexAuthStore
  fetch?: (
    input: string | URL | Request,
    init?: RequestInit
  ) => Promise<Response>
  userAgent?: string
  /** Where the latest snapshot is kept across rebuilds and restarts (`createProviderQuota`). */
  snapshots?: ProviderQuotaSnapshots
}

const DEFAULT_CHATGPT_BASE_URL = 'https://chatgpt.com/backend-api'

/** The account's usage status, as the ChatGPT backend answers `GET /wham/usage`. */
const usageWindowSchema = z.object({
  used_percent: z.number(),
  limit_window_seconds: z.number(),
  reset_at: z.number(),
}).loose()
const usageStatusSchema = z.object({
  plan_type: z.string().optional(),
  rate_limit: z.object({
    primary_window: usageWindowSchema.nullish(),
    secondary_window: usageWindowSchema.nullish(),
  }).loose().nullish(),
}).loose()

/**
 * Codex consumer rate windows, from two places that agree:
 * - probe(): the account's usage status, `GET …/wham/usage`, which the
 *   open-source Codex CLI reads for its own status. It spends nothing and
 *   answers while the limit is reached, when a request would be refused.
 * - observeResponse: `x-codex-{primary,secondary}-used-percent` /
 *   `-window-minutes` / `-reset-at` on any live Responses call.
 * The usage status belongs to a ChatGPT sign-in; an API key has none.
 */
export function createCodexQuota(options: CodexQuotaOptions = {}): ProviderQuota {
  const providerId = options.providerId ?? 'codex'
  const authStore = options.authStore ?? new FileCodexAuthStore({
    codexHome: options.codexHome
  })
  const fetchImpl = options.fetch ?? fetch

  return createProviderQuota({
    providerId,
    snapshots: options.snapshots,
    canProbe: true,
    canObserve: true,
    probeCost: 'free',
    staleAfterMs: 5 * 60_000,
    probe: async ({ signal } = {}) => {
      const auth = await authStore.resolveAuth()
      if (auth.kind === 'apiKey')
        throw new Error('An OpenAI API key has no Codex usage status; its windows come from responses')
      const status = await authStore.status()
      const accountLabel = status.status === 'authenticated'
        ? status.accountLabel ?? null
        : null
      const headers = buildCodexHeaders(
        auth,
        { sessionId: 'demi-codex-quota-probe', requestId: randomUUID() },
        { userAgent: options.userAgent },
      )
      const base = (options.baseUrl ?? DEFAULT_CHATGPT_BASE_URL).replace(/\/+$/, '')
      const response = await fetchImpl(`${base}/wham/usage`, { method: 'GET', headers, signal })
      if (!response.ok) {
        await response.body?.cancel().catch(() => {})
        throw new Error(`Codex usage request failed (HTTP ${response.status})`)
      }
      const usage = usageStatusSchema.parse(await response.json())
      return {
        ...mapCodexUsageStatus(usage),
        accountLabel,
        raw: usage,
      }
    },
    observe: ({ headers }) => mapCodexRateLimitHeaders(headers),
  })
}

/** The plan and the two windows of a usage status. */
export function mapCodexUsageStatus(usage: z.infer<typeof usageStatusSchema>): ProviderQuotaProbeResult {
  const windows = (['primary', 'secondary'] as const).flatMap((kind) => {
    const window = usage.rate_limit?.[`${kind}_window`]
    if (!window)
      return []
    const usedPercent = clampUsedPercent(window.used_percent)
    return [{
      id: kind,
      label: windowLabel(kind, window.limit_window_seconds / 60),
      usedPercent,
      unit: 'percent' as const,
      resetsAt: unixSecondsToIso(window.reset_at),
      severity: severityFromUsedPercent(usedPercent),
    }]
  })
  const plan = usage.plan_type
  return {
    windows,
    plan: plan ? { id: plan, label: planLabel(plan), raw: plan } : null,
  }
}

/** `plus` reads Plus; `self_serve_business_usage_based` reads Self serve business usage based. */
function planLabel(plan: string): string {
  const words = plan.replaceAll('_', ' ')
  return words.charAt(0).toUpperCase() + words.slice(1)
}

/** A window is named by its length, which is what the user knows it by; the vendor's own word for it otherwise. */
function windowLabel(kind: 'primary' | 'secondary', minutes: number | null): string {
  const name = kind === 'primary' ? 'Primary' : 'Secondary'
  if (minutes === null || !(minutes > 0))
    return name
  if (minutes % (7 * 24 * 60) === 0)
    return minutes === 7 * 24 * 60 ? 'Weekly' : `${minutes / (7 * 24 * 60)}-week`
  if (minutes % (24 * 60) === 0)
    return minutes === 24 * 60 ? 'Daily' : `${minutes / (24 * 60)}-day`
  if (minutes % 60 === 0)
    return `${minutes / 60}-hour`
  return `${minutes}-minute`
}

export function mapCodexRateLimitHeaders(
  headers: Headers | undefined
): ProviderQuotaProbeResult | null {
  if (!headers)
    return null
  const primary = parseCodexWindow(headers, 'primary')
  const secondary = parseCodexWindow(headers, 'secondary')
  const windows = [primary, secondary].filter((w): w is ProviderQuotaWindow => w !== null)
  if (windows.length === 0)
    return null
  return {
    windows,
    raw: {
      primary: headerBag(headers, 'primary'),
      secondary: headerBag(headers, 'secondary'),
    },
  }
}

function parseCodexWindow(
  headers: Headers,
  kind: 'primary' | 'secondary'
): ProviderQuotaWindow | null {
  const usedPercent = clampUsedPercent(numberHeader(
    headers,
    `x-codex-${kind}-used-percent`
  ))
  if (usedPercent == null && !headers.has(`x-codex-${kind}-used-percent`))
    return null
  const windowMinutes = numberHeader(headers, `x-codex-${kind}-window-minutes`)
  const resetAt = unixSecondsToIso(numberHeader(
    headers,
    `x-codex-${kind}-reset-at`
  ))
  // The same names the usage status gives, so a window keeps its name whichever source saw it last.
  const label = windowLabel(kind, windowMinutes)
  return {
    id: kind,
    label,
    usedPercent,
    unit: 'percent',
    resetsAt: resetAt,
    severity: severityFromUsedPercent(usedPercent),
  }
}

function headerBag(
  headers: Headers,
  kind: 'primary' | 'secondary'
): Record<string, string | null> {
  return {
    usedPercent: headers.get(`x-codex-${kind}-used-percent`),
    windowMinutes: headers.get(`x-codex-${kind}-window-minutes`),
    resetAt: headers.get(`x-codex-${kind}-reset-at`),
  }
}

