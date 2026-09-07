import { randomUUID } from 'node:crypto'
import { isRecord, nonEmptyString, numberOrNull } from '@demicodes/utils'
import {
  ProviderQuotaUnsupportedError,
  clampUsedPercent,
  numberHeader,
  createProviderQuota,
  severityFromUsedPercent,
  unixSecondsToIso,
  type ProviderQuota,
  type ProviderQuotaProbeResult,
  type ProviderQuotaWindow,
} from '@demicodes/provider'
import { FileCodexAuthStore, type CodexAuthStore } from './auth'
import { buildCodexHeaders } from './provider'

export interface CodexQuotaOptions {
  providerId?: string
  codexHome?: string
  baseUrl?: string
  authStore?: CodexAuthStore
  fetch?: (input: string | URL | Request, init?: RequestInit) => Promise<Response>
  userAgent?: string
}

/** Probes the Codex usage endpoint; live inference still observes rate-limit headers. */
export function createCodexQuota(options: CodexQuotaOptions = {}): ProviderQuota {
  const providerId = options.providerId ?? 'codex'
  const authStore = options.authStore ?? new FileCodexAuthStore({ codexHome: options.codexHome })
  const fetchImpl = options.fetch ?? fetch

  return createProviderQuota({
    providerId,
    canProbe: true,
    canObserve: true,
    probeCost: 'free',
    staleAfterMs: 5 * 60_000,
    probe: async ({ signal } = {}) => {
      const request = async (forceRefresh = false): Promise<Response> => {
        signal?.throwIfAborted()
        const auth = await authStore.resolveAuth({ forceRefresh })
        if (auth.kind === 'apiKey') {
          throw new ProviderQuotaUnsupportedError(providerId, 'Codex usage requires ChatGPT account authentication')
        }
        const headers = buildCodexHeaders(
          auth,
          { sessionId: 'demi-codex-quota-probe', requestId: randomUUID() },
          { userAgent: options.userAgent },
        )
        headers.set('accept', 'application/json')
        const baseUrl = (options.baseUrl ?? 'https://chatgpt.com/backend-api')
          .replace(/\/+$/, '').replace(/\/codex(?:\/responses)?$/, '')
        return fetchImpl(`${baseUrl}/wham/usage`, { method: 'GET', headers, signal })
      }
      let response = await request()
      if (response.status === 401) {
        await response.body?.cancel()
        response = await request(true)
      }
      if (!response.ok) {
        await response.body?.cancel()
        throw new Error(`Codex usage request failed with HTTP ${response.status}`)
      }
      const partial = mapCodexUsagePayload(await response.json())
      const status = await authStore.status()
      return { ...partial, accountLabel: status.status === 'authenticated' ? status.accountLabel ?? null : null }

    },
    observe: ({ headers }) => mapCodexRateLimitHeaders(headers),
  })
}

function mapCodexUsagePayload(payload: unknown): ProviderQuotaProbeResult {
  if (!isRecord(payload) || typeof payload.plan_type !== 'string') {
    throw new Error('Codex usage response has invalid quota data')
  }
  const windows = mapUsageWindows(payload.rate_limit)
  if (Array.isArray(payload.additional_rate_limits)) {
    for (const limit of payload.additional_rate_limits) {
      if (!isRecord(limit)) continue
      const id = nonEmptyString(limit.metered_feature)
      const label = nonEmptyString(limit.limit_name)
      if (!id || !label) continue
      windows.push(...mapUsageWindows(limit.rate_limit).map((window) => ({
        ...window, id: `${id}:${window.id}`, scope: { kind: 'model', label },
      })))
    }
  }
  return { windows, plan: { id: payload.plan_type, label: payload.plan_type } }
}

function mapUsageWindows(value: unknown): ProviderQuotaWindow[] {
  if (!isRecord(value)) return []
  const headers = new Headers()
  for (const kind of ['primary', 'secondary'] as const) {
    const window = value[`${kind}_window`]
    if (!isRecord(window)) continue
    const usedPercent = numberOrNull(window.used_percent)
    if (usedPercent === null) continue
    headers.set(`x-codex-${kind}-used-percent`, String(usedPercent))
    const seconds = numberOrNull(window.limit_window_seconds)
    if (seconds !== null) headers.set(`x-codex-${kind}-window-minutes`, String(seconds / 60))
    const resetAt = numberOrNull(window.reset_at)
    if (resetAt !== null) headers.set(`x-codex-${kind}-reset-at`, String(resetAt))
  }
  return mapCodexRateLimitHeaders(headers)?.windows ?? []
}

export function mapCodexRateLimitHeaders(headers: Headers | undefined): ProviderQuotaProbeResult | null {
  if (!headers) return null
  const primary = parseCodexWindow(headers, 'primary')
  const secondary = parseCodexWindow(headers, 'secondary')
  const windows = [primary, secondary].filter((w): w is ProviderQuotaWindow => w !== null)
  if (windows.length === 0) return null
  return {
    windows,
    raw: {
      primary: headerBag(headers, 'primary'),
      secondary: headerBag(headers, 'secondary'),
    },
  }
}

function parseCodexWindow(headers: Headers, kind: 'primary' | 'secondary'): ProviderQuotaWindow | null {
  const usedPercent = clampUsedPercent(numberHeader(headers, `x-codex-${kind}-used-percent`))
  if (usedPercent == null && !headers.has(`x-codex-${kind}-used-percent`)) return null
  const windowMinutes = numberHeader(headers, `x-codex-${kind}-window-minutes`)
  const resetAt = unixSecondsToIso(numberHeader(headers, `x-codex-${kind}-reset-at`))
  const label =
    kind === 'primary'
      ? windowMinutes != null
        ? `Primary (${windowMinutes}m)`
        : 'Primary'
      : windowMinutes != null
        ? `Secondary (${windowMinutes}m)`
        : 'Secondary'
  return {
    id: kind,
    label,
    usedPercent,
    unit: 'percent',
    resetsAt: resetAt,
    severity: severityFromUsedPercent(usedPercent),
  }
}

function headerBag(headers: Headers, kind: 'primary' | 'secondary'): Record<string, string | null> {
  return {
    usedPercent: headers.get(`x-codex-${kind}-used-percent`),
    windowMinutes: headers.get(`x-codex-${kind}-window-minutes`),
    resetAt: headers.get(`x-codex-${kind}-reset-at`),
  }
}

