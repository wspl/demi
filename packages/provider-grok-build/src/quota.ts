import { grokQuotaUserSchema, grokQuotaBillingSchema, grokRateLimitSchema } from './quota-schemas'
import {
  clampUsedPercent,
  parseProviderData,
  createProviderQuota,
  ProviderDataError,
  numberHeader,
  severityFromUsedPercent,
  usedPercentFromRatio,
  type ProviderQuota,
  type ProviderQuotaProbeResult,
  type ProviderQuotaWindow,
} from '@demicodes/provider'
import type { GrokAuthStore, GrokResolvedAuth } from './auth'
import { FileGrokAuthStore } from './auth'
import {
  DEFAULT_GROK_BUILD_BASE_URL,
  buildGrokBuildHeaders,
  resolveGrokClientVersion,
} from './headers'
import type { GrokBuildFetch } from './provider'

export interface GrokBuildQuotaOptions {
  providerId?: string
  grokHome?: string
  baseUrl?: string
  clientVersion?: string
  authStore?: GrokAuthStore
  fetch?: GrokBuildFetch
}

/**
 * Active probe against cli-chat-proxy:
 * - GET /v1/user?include=subscription → tier
 * - GET /v1/billing?format=credits → creditUsagePercent / currentPeriod
 *
 * Optional observation of short-window x-ratelimit-* headers from chat
 * responses
 * (separate windows from monthly subscription quota).
 */
export function createGrokBuildQuota(
  options: GrokBuildQuotaOptions = {}
): ProviderQuota {
  const providerId = options.providerId ?? 'grok-build'
  const authStore = options.authStore ?? new FileGrokAuthStore({
    grokHome: options.grokHome
  })
  const baseUrl = (options.baseUrl ?? DEFAULT_GROK_BUILD_BASE_URL).replace(
    /\/+$/,
    ''
  )
  const fetchImpl: GrokBuildFetch = options.fetch ?? ((input, init) => fetch(
    input,
    init
  ))
  const grokHome = options.grokHome
  const clientVersion = options.clientVersion

  return createProviderQuota({
    providerId,
    canProbe: true,
    canObserve: true,
    probeCost: 'free',
    staleAfterMs: 60_000,
    probe: async ({ signal } = {}) => {
      const auth = await authStore.resolveAuth()
      const [user, billing] = await Promise.all([
        fetchJson(
          fetchImpl,
          `${baseUrl}/user?include=subscription`,
          auth,
          { grokHome, clientVersion },
          signal
        ),
        fetchJson(
          fetchImpl,
          `${baseUrl}/billing?format=credits`,
          auth,
          { grokHome, clientVersion },
          signal
        ),
      ])
      return mapGrokQuotaProbe(user, billing, auth)
    },
    observe: ({ headers }) => observeGrokRateLimitHeaders(headers),
  })
}

export function mapGrokQuotaProbe(
  user: unknown,
  billing: unknown,
  auth?: Pick<GrokResolvedAuth, 'email'>,
): ProviderQuotaProbeResult {
  const userRecord = parseProviderData(grokQuotaUserSchema, user, 'Grok quota user')
  const billingRecord = parseProviderData(grokQuotaBillingSchema, billing, 'Grok quota billing')
  const config = billingRecord.config

  const period = config?.currentPeriod
  const periodType = period?.type
  const isWeekly = periodType === 'USAGE_PERIOD_TYPE_WEEKLY'
  const isMonthly = periodType === 'USAGE_PERIOD_TYPE_MONTHLY'
    || (periodType === undefined && config?.monthlyLimit != null)
  const resetsAt = period?.end ?? config?.billingPeriodEnd ?? null
  const monthlyLimit = config?.monthlyLimit ?? null
  const used = config?.used ?? null
  const onDemandCap = config?.onDemandCap ?? null
  const usedPercent = clampUsedPercent(config?.creditUsagePercent)
    ?? usedPercentFromRatio(used, monthlyLimit)

  const windows: ProviderQuotaWindow[] = config ? [
    {
      id: isWeekly ? 'weekly' : isMonthly ? 'monthly' : 'credits',
      label: isWeekly ? 'Weekly credits' : isMonthly ? 'Monthly credits' : 'Credits',
      usedPercent,
      used,
      limit: monthlyLimit,
      unit: 'credits',
      resetsAt,
      severity: severityFromUsedPercent(usedPercent),
    },
  ] : []

  if (onDemandCap != null && onDemandCap > 0) {
    windows.push({
      id: 'on_demand_cap',
      label: 'On-demand cap',
      usedPercent: null,
      used: null,
      limit: onDemandCap,
      unit: 'credits',
      resetsAt,
    })
  }

  const tier = userRecord.subscriptionTier
  return {
    plan: tier ? { id: tier, label: tier, raw: tier } : null,
    accountLabel: auth?.email ?? userRecord.email ?? null,
    windows,
    raw: { user, billing },
  }
}

/** Short-window chat ratelimits — not subscription monthly quota. */
export function observeGrokRateLimitHeaders(
  headers: Headers | undefined
): ProviderQuotaProbeResult | null {
  if (!headers)
    return null
  const windows: ProviderQuotaWindow[] = []
  for (const kind of ['requests', 'tokens'] as const) {
    const { remaining, limit } = parseProviderData(grokRateLimitSchema, {
      remaining: numberHeader(headers, `x-ratelimit-remaining-${kind}`),
      limit: numberHeader(headers, `x-ratelimit-limit-${kind}`),
    }, `Grok ${kind} quota`)
    if (limit === null) {
      continue
    }
    const used = remaining === null ? null : limit - remaining
    const usedPercent = usedPercentFromRatio(used, limit)
    windows.push({
      id: kind === 'requests' ? 'rpm' : 'tpm',
      label: kind === 'requests' ? 'Requests (short window)' : 'Tokens (short window)',
      usedPercent,
      used,
      limit,
      unit: kind,
      resetsAt: null,
      severity: severityFromUsedPercent(usedPercent),
    })
  }
  return windows.length > 0 ? { windows } : null
}

async function fetchJson(
  fetchImpl: GrokBuildFetch,
  url: string,
  auth: GrokResolvedAuth,
  opts: {
    grokHome?: string;
    clientVersion?: string
  },
  signal?: AbortSignal,
): Promise<unknown> {
  const headers = buildGrokBuildHeaders(auth, undefined, {
    clientVersion: opts.clientVersion ?? resolveGrokClientVersion(
      undefined,
      opts.grokHome
    ),
    grokHome: opts.grokHome,
  })
  headers.set('accept', 'application/json')
  const response = await fetchImpl(url, { method: 'GET', headers, signal })
  if (!response.ok) {
    await response.body?.cancel().catch(() => {
      // An errored response body is already closed.
    })
    throw new Error(
      `Grok quota request failed (HTTP ${response.status})`
    )
  }
  try {
    return await response.json()
  } catch (error) {
    signal?.throwIfAborted()
    if (error instanceof SyntaxError) {
      throw new ProviderDataError('Grok quota', 'invalid JSON')
    }
    throw error
  }
}

