import { isRecord, stringOrNull } from '@demicodes/utils'
import {
  createProviderQuota,
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
 * - GET /v1/billing → monthly used/limit
 *
 * Optional observation of short-window x-ratelimit-* headers from chat responses
 * (separate windows from monthly subscription quota).
 */
export function createGrokBuildQuota(options: GrokBuildQuotaOptions = {}): ProviderQuota {
  const providerId = options.providerId ?? 'grok-build'
  const authStore = options.authStore ?? new FileGrokAuthStore({ grokHome: options.grokHome })
  const baseUrl = (options.baseUrl ?? DEFAULT_GROK_BUILD_BASE_URL).replace(/\/+$/, '')
  const fetchImpl: GrokBuildFetch = options.fetch ?? ((input, init) => fetch(input, init))
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
        fetchJson(fetchImpl, `${baseUrl}/user?include=subscription`, auth, { grokHome, clientVersion }, signal),
        fetchJson(fetchImpl, `${baseUrl}/billing`, auth, { grokHome, clientVersion }, signal),
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
  const userRecord = isRecord(user) ? user : {}
  const billingRecord = isRecord(billing) ? billing : {}
  const config = isRecord(billingRecord.config) ? billingRecord.config : {}

  const monthlyLimit = moneyVal(config.monthlyLimit)
  const used = moneyVal(config.used)
  const onDemandCap = moneyVal(config.onDemandCap)
  const usedPercent = usedPercentFromRatio(used, monthlyLimit)

  const windows: ProviderQuotaWindow[] = [
    {
      id: 'monthly',
      label: 'Monthly credits',
      usedPercent,
      used,
      limit: monthlyLimit,
      unit: 'credits',
      resetsAt: stringOrNull(config.billingPeriodEnd),
      severity: severityFromUsedPercent(usedPercent),
    },
  ]

  if (onDemandCap != null && onDemandCap > 0) {
    windows.push({
      id: 'on_demand_cap',
      label: 'On-demand cap',
      usedPercent: null,
      used: null,
      limit: onDemandCap,
      unit: 'credits',
      resetsAt: stringOrNull(config.billingPeriodEnd),
    })
  }

  const tier = stringOrNull(userRecord.subscriptionTier)
  return {
    plan: tier ? { id: tier, label: tier, raw: tier } : null,
    accountLabel: auth?.email ?? stringOrNull(userRecord.email),
    windows,
    raw: { user, billing },
  }
}

/** Short-window chat ratelimits — not subscription monthly quota. */
export function observeGrokRateLimitHeaders(headers: Headers | undefined): ProviderQuotaProbeResult | null {
  if (!headers) return null
  const remReq = numberHeader(headers, 'x-ratelimit-remaining-requests')
  const limReq = numberHeader(headers, 'x-ratelimit-limit-requests')
  const remTok = numberHeader(headers, 'x-ratelimit-remaining-tokens')
  const limTok = numberHeader(headers, 'x-ratelimit-limit-tokens')
  if (remReq == null && limReq == null && remTok == null && limTok == null) return null

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
  opts: { grokHome?: string; clientVersion?: string },
  signal?: AbortSignal,
): Promise<unknown> {
  const headers = buildGrokBuildHeaders(auth, undefined, {
    clientVersion: opts.clientVersion ?? resolveGrokClientVersion(undefined, opts.grokHome),
    grokHome: opts.grokHome,
  })
  headers.set('accept', 'application/json')
  const response = await fetchImpl(url, { method: 'GET', headers, signal })
  if (!response.ok) {
    const body = await response.text().catch(() => '')
    throw new Error(`Grok quota request failed (${response.status}): ${body.slice(0, 200)}`)
  }
  return response.json()
}

function moneyVal(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return value
  if (isRecord(value) && typeof value.val === 'number' && Number.isFinite(value.val)) return value.val
  return null
}

function numberHeader(headers: Headers, name: string): number | null {
  const raw = headers.get(name)
  if (raw == null || raw === '') return null
  const n = Number(raw)
  return Number.isFinite(n) ? n : null
}

