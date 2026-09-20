import { z } from 'zod'
import {
  clampUsedPercent,
  createProviderQuota,
  numberHeader,
  reportedStringSchema,
  severityFromUsedPercent,
  usedPercentFromRatio,
  type ProviderQuota,
  type ProviderQuotaProbeResult,
  type ProviderQuotaSnapshots,
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

/**
 * The cli-chat-proxy quota payloads, as the quota surface reads them. Quota is
 * a display surface: a field the vendor spells in a shape Demi does not know
 * is dropped, and the rest of the snapshot still renders.
 */
const creditAmountSchema = z
  .union([
    z.number().finite(),
    z.looseObject({ val: z.number().finite() }).transform((money) => money.val),
  ])
  .nullable()
  .catch(null)

const grokBillingSchema = z
  .looseObject({
    config: z
      .looseObject({
        creditUsagePercent: z.number().optional().catch(undefined),
        currentPeriod: z
          .looseObject({ type: reportedStringSchema, end: reportedStringSchema })
          .optional()
          .catch(undefined),
        billingPeriodEnd: reportedStringSchema,
        monthlyLimit: creditAmountSchema,
        used: creditAmountSchema,
        onDemandCap: creditAmountSchema,
      })
      .optional()
      .catch(undefined),
  })
  .catch({})

const grokUserSchema = z
  .looseObject({
    subscriptionTier: reportedStringSchema,
    email: reportedStringSchema,
  })
  .catch({})

const WEEKLY_PERIOD = 'USAGE_PERIOD_TYPE_WEEKLY'

export interface GrokBuildQuotaOptions {
  providerId?: string
  grokHome?: string
  baseUrl?: string
  clientVersion?: string
  authStore?: GrokAuthStore
  fetch?: GrokBuildFetch
  /** Where the latest snapshot is kept across rebuilds and restarts (`createProviderQuota`). */
  snapshots?: ProviderQuotaSnapshots
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
    snapshots: options.snapshots,
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
  const account = grokUserSchema.parse(user)
  const config = grokBillingSchema.parse(billing).config

  const period = config?.currentPeriod
  const isWeekly = period?.type === WEEKLY_PERIOD
  const resetsAt = period?.end ?? config?.billingPeriodEnd ?? null
  const monthlyLimit = config?.monthlyLimit ?? null
  const used = config?.used ?? null
  const onDemandCap = config?.onDemandCap ?? null
  const usedPercent = clampUsedPercent(config?.creditUsagePercent)
    ?? usedPercentFromRatio(used, monthlyLimit)

  const windows: ProviderQuotaWindow[] = []
  // A plan that meters no credits names none of these; a window with nothing to show is not one.
  if (usedPercent !== null || used !== null || monthlyLimit !== null) {
    windows.push({
      id: isWeekly ? 'weekly' : 'monthly',
      label: isWeekly ? 'Weekly credits' : 'Monthly credits',
      usedPercent,
      used,
      limit: monthlyLimit,
      unit: 'credits',
      resetsAt,
      severity: severityFromUsedPercent(usedPercent),
    })
  }

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

  const tier = account.subscriptionTier
  return {
    plan: tier ? { id: tier, label: tier, raw: tier } : null,
    accountLabel: auth?.email ?? account.email ?? null,
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
  const remReq = numberHeader(headers, 'x-ratelimit-remaining-requests')
  const limReq = numberHeader(headers, 'x-ratelimit-limit-requests')
  const remTok = numberHeader(headers, 'x-ratelimit-remaining-tokens')
  const limTok = numberHeader(headers, 'x-ratelimit-limit-tokens')
  if (remReq == null && limReq == null && remTok == null && limTok == null)
    return null

  const windows: ProviderQuotaWindow[] = []
  if (limReq != null) {
    const used = remReq != null ? limReq - remReq : null
    const usedPercent = usedPercentFromRatio(used, limReq)
    windows.push({
      id: 'rpm',
      label: 'Requests (short window)',
      usedPercent,
      used,
      limit: limReq,
      unit: 'requests',
      resetsAt: null,
      severity: severityFromUsedPercent(usedPercent),
    })
  }
  if (limTok != null) {
    const used = remTok != null ? limTok - remTok : null
    const usedPercent = usedPercentFromRatio(used, limTok)
    windows.push({
      id: 'tpm',
      label: 'Tokens (short window)',
      usedPercent,
      used,
      limit: limTok,
      unit: 'tokens',
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
    const body = await response.text().catch(() => '')
    throw new Error(
      `Grok quota request failed (${response.status}): ${body.slice(0, 200)}`
    )
  }
  return response.json()
}
