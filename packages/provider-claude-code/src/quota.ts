import { isRecord, stringOrNull } from '@demicodes/utils'
import {
  clampUsedPercent,
  createProviderQuota,
  severityFromUsedPercent,
  unixSecondsToIso,
  type ProviderQuota,
  type ProviderQuotaProbeResult,
  type ProviderQuotaWindow,
} from '@demicodes/provider'
import { resolveClaudeCodeOAuthAccess, type ClaudeCodeOAuthAccess } from './oauth'

export interface ClaudeCodeQuotaOptions {
  providerId?: string
  /** Override token resolution (tests / custom stores). */
  resolveAccess?: () => Promise<ClaudeCodeOAuthAccess | null>
  fetch?: (input: string | URL | Request, init?: RequestInit) => Promise<Response>
  usageUrl?: string
}

const DEFAULT_USAGE_URL = 'https://api.anthropic.com/api/oauth/usage'
const DEFAULT_OAUTH_BETA = 'oauth-2025-04-20'

/**
 * Active probe: GET /api/oauth/usage (Claude.ai consumer plan windows).
 * Observation: anthropic-ratelimit-unified-* response headers.
 */
export function createClaudeCodeQuota(options: ClaudeCodeQuotaOptions = {}): ProviderQuota {
  const providerId = options.providerId ?? 'claude-code'
  const fetchImpl = options.fetch ?? fetch
  const usageUrl = options.usageUrl ?? DEFAULT_USAGE_URL
  const resolveAccess = options.resolveAccess ?? resolveClaudeCodeOAuthAccess

  return createProviderQuota({
    providerId,
    canProbe: true,
    canObserve: true,
    probeCost: 'free',
    staleAfterMs: 60_000,
    probe: async ({ signal } = {}) => {
      const access = await resolveAccess()
      if (!access?.accessToken) {
        throw new Error(
          'Claude Code OAuth access token not found (set CLAUDE_CODE_OAUTH_TOKEN or log in with Claude Code)',
        )
      }
      const headers = new Headers({
        authorization: `Bearer ${access.accessToken}`,
        'anthropic-beta': DEFAULT_OAUTH_BETA,
        accept: 'application/json',
        'user-agent': 'demi-provider-claude-code',
      })
      const response = await fetchImpl(usageUrl, { method: 'GET', headers, signal })
      if (!response.ok) {
        const body = await response.text().catch(() => '')
        throw new Error(`Claude usage request failed (${response.status}): ${body.slice(0, 200)}`)
      }
      const payload = await response.json()
      return mapClaudeUsagePayload(payload, access)
    },
    observe: ({ headers, body }) => {
      if (headers) {
        const fromHeaders = observeClaudeRateLimitHeaders(headers)
        if (fromHeaders) return fromHeaders
      }
      if (body !== undefined) return observeClaudeStreamBody(body)
      return null
    },
  })
}

/** Claude CLI stream-json / status envelopes that embed `rate_limits`. */
export function observeClaudeStreamBody(body: unknown): ProviderQuotaProbeResult | null {
  if (!isRecord(body)) return null
  const rateLimits = isRecord(body.rate_limits)
    ? body.rate_limits
    : isRecord(body.message) && isRecord(body.message.rate_limits)
      ? body.message.rate_limits
      : null
  if (!rateLimits) return null
  // Reuse payload mapper shape: five_hour / seven_day on the rate_limits object.
  const partial = mapClaudeUsagePayload(rateLimits)
  return partial.windows.length > 0 ? { windows: partial.windows, raw: rateLimits } : null
}

export function mapClaudeUsagePayload(
  payload: unknown,
  access?: ClaudeCodeOAuthAccess | null,
): ProviderQuotaProbeResult {
  const record = isRecord(payload) ? payload : {}
  const windows: ProviderQuotaWindow[] = []

  pushWindow(windows, 'five_hour', '5h session', record.five_hour)
  pushWindow(windows, 'seven_day', '7d all models', record.seven_day)
  pushWindow(windows, 'seven_day_sonnet', '7d Sonnet', record.seven_day_sonnet)
  pushWindow(windows, 'seven_day_opus', '7d Opus', record.seven_day_opus)

  if (Array.isArray(record.limits)) {
    for (const item of record.limits) {
      if (!isRecord(item)) continue
      const kind = typeof item.kind === 'string' ? item.kind : null
      if (!kind) continue
      // Prefer dedicated five_hour/seven_day objects when present.
      if (kind === 'session' || kind === 'weekly_all') continue
      const percent = clampUsedPercent(typeof item.percent === 'number' ? item.percent : null)
      const scopeLabel =
        isRecord(item.scope) && isRecord(item.scope.model) && typeof item.scope.model.display_name === 'string'
          ? item.scope.model.display_name
          : undefined
      windows.push({
        id: `limit:${kind}${scopeLabel ? `:${scopeLabel}` : ''}`,
        label: scopeLabel ? `${kind} (${scopeLabel})` : kind,
        usedPercent: percent,
        unit: 'percent',
        resetsAt: unixSecondsToIso(item.resets_at) ?? stringOrNull(item.resets_at),
        severity:
          item.severity === 'critical' || item.severity === 'warning' || item.severity === 'normal'
            ? item.severity
            : severityFromUsedPercent(percent),
        scope: scopeLabel ? { kind: 'model', label: scopeLabel } : { kind },
      })
    }
  }

  const planId = access?.subscriptionType ?? null
  return {
    plan: planId ? { id: planId, label: planId, raw: access?.rateLimitTier ?? planId } : null,
    accountLabel: null,
    windows,
    raw: payload,
  }
}

/** Map anthropic-ratelimit-unified-* headers into a coarse snapshot. */
export function observeClaudeRateLimitHeaders(headers: Headers | undefined): ProviderQuotaProbeResult | null {
  if (!headers) return null
  const status = headers.get('anthropic-ratelimit-unified-status')
  const reset = headers.get('anthropic-ratelimit-unified-reset')
  const claim = headers.get('anthropic-ratelimit-unified-representative-claim')
  const overageUtil = headers.get('anthropic-ratelimit-unified-overage-period-channel-utilization')
  if (!status && !reset && !claim && !overageUtil) return null

  const usedPercent = clampUsedPercent(overageUtil != null ? Number(overageUtil) : null)
  const windows: ProviderQuotaWindow[] = [
    {
      id: 'unified',
      label: claim ?? 'Unified rate limit',
      usedPercent,
      unit: 'percent',
      resetsAt: unixSecondsToIso(reset),
      severity:
        status === 'rejected' || status === 'allowed_warning'
          ? status === 'rejected'
            ? 'critical'
            : 'warning'
          : severityFromUsedPercent(usedPercent),
    },
  ]
  return { windows, raw: { status, reset, claim, overageUtil } }
}

function pushWindow(
  windows: ProviderQuotaWindow[],
  id: string,
  label: string,
  value: unknown,
): void {
  if (!isRecord(value)) return
  const usedPercent = clampUsedPercent(
    typeof value.utilization === 'number' ? value.utilization : typeof value.used_percentage === 'number' ? value.used_percentage : null,
  )
  windows.push({
    id,
    label,
    usedPercent,
    unit: 'percent',
    resetsAt: unixSecondsToIso(value.resets_at) ?? stringOrNull(value.resets_at),
    severity: severityFromUsedPercent(usedPercent),
  })
}

