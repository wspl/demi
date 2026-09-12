import { randomUUID } from 'node:crypto'
import { codexQuotaWindowSchema } from './quota-schemas'
import {
  clampUsedPercent,
  parseProviderData,
  numberHeader,
  createProviderQuota,
  severityFromUsedPercent,
  unixSecondsToIso,
  type ProviderQuota,
  type ProviderQuotaWindow,
} from '@demicodes/provider'
import { FileCodexAuthStore, type CodexAuthStore } from './auth'
import { buildCodexHeaders, responsesUrlForAuth } from './provider'

export interface CodexQuotaOptions {
  providerId?: string
  codexHome?: string
  baseUrl?: string
  authStore?: CodexAuthStore
  /**
   * Model id for the minimal probe request (must be accepted by the backend).
   */
  probeModelId?: string
  fetch?: (
    input: string | URL | Request,
    init?: RequestInit
  ) => Promise<Response>
  userAgent?: string
}

const DEFAULT_PROBE_MODEL = 'gpt-5.4'

/**
 * Codex consumer rate windows come from Responses headers:
 * x-codex-{primary,secondary}-used-percent / -window-minutes / -reset-at
 *
 * probe() issues a minimal streamed Responses request and cancels the body
 * (probeCost: minimal_request). observeResponse can parse the same headers
 * from any live Responses call without an extra request.
 */
export function createCodexQuota(options: CodexQuotaOptions = {}): ProviderQuota {
  const providerId = options.providerId ?? 'codex'
  const authStore = options.authStore ?? new FileCodexAuthStore({
    codexHome: options.codexHome
  })
  const fetchImpl = options.fetch ?? fetch
  const probeModelId = options.probeModelId ?? DEFAULT_PROBE_MODEL

  return createProviderQuota({
    providerId,
    canProbe: true,
    canObserve: true,
    probeCost: 'minimal_request',
    staleAfterMs: 5 * 60_000,
    probe: async ({ signal } = {}) => {
      const auth = await authStore.resolveAuth()
      const status = await authStore.status()
      const accountLabel = status.status === 'authenticated'
        ? status.accountLabel ?? null
        : null
      const headers = buildCodexHeaders(
        auth,
        { sessionId: 'demi-codex-quota-probe', requestId: randomUUID() },
        { userAgent: options.userAgent },
      )
      const body = {
        model: probeModelId,
        instructions: 'Reply with pong.',
        input: [{
          type: 'message',
          role: 'user',
          content: [{ type: 'input_text', text: 'ping' }]
        }],
        tools: [],
        tool_choice: 'auto',
        parallel_tool_calls: true,
        store: false,
        stream: true,
        include: [],
        prompt_cache_key: 'demi-codex-quota-probe',
        text: { verbosity: 'low' },
      }
      const response = await fetchImpl(responsesUrlForAuth(
        auth,
        options.baseUrl
      ), {
        method: 'POST',
        headers,
        body: JSON.stringify(body),
        signal,
      })
      let partial: ReturnType<typeof mapCodexRateLimitHeaders>
      try {
        partial = mapCodexRateLimitHeaders(response.headers)
      } finally {
        await response.body?.cancel().catch(() => {
          // An errored body is already closed; cancellation only releases the probe stream.
        })
      }
      if (!partial) {
        throw new Error(
          `Codex quota probe got no rate-limit headers (HTTP ${response.status})`
        )
      }
      return {
        ...partial,
        accountLabel,
        raw: {
          ...partial.raw,
          httpStatus: response.status,
          authKind: auth.kind
        },
      }
    },
    observe: ({ headers }) => mapCodexRateLimitHeaders(headers),
  })
}

export function mapCodexRateLimitHeaders(
  headers: Headers | undefined
) {
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
  const window = parseProviderData(codexQuotaWindowSchema, {
    usedPercent: numberHeader(headers, `x-codex-${kind}-used-percent`),
    windowMinutes: numberHeader(headers, `x-codex-${kind}-window-minutes`),
    resetSeconds: numberHeader(headers, `x-codex-${kind}-reset-at`),
  }, `Codex ${kind} quota`)
  const usedPercent = clampUsedPercent(window.usedPercent)
  const resetAt = unixSecondsToIso(window.resetSeconds)
  if (usedPercent === null && window.windowMinutes === null && resetAt === null) {
    return null
  }
  const baseLabel = kind === 'primary' ? 'Primary' : 'Secondary'
  const label = window.windowMinutes === null
    ? baseLabel
    : `${baseLabel} (${window.windowMinutes}m)`
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

