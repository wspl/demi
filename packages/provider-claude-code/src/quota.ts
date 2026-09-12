import {
  claudeQuotaEnvelopeSchema, claudeQuotaPayloadSchema,
  type ClaudeQuotaPayload, type ClaudeQuotaWindow,
} from './quota-schemas'
import {
  clampUsedPercent,
  createProviderQuota,
  numberHeader,
  severityFromUsedPercent,
  parseProviderData,
  parseProviderJson,
  quotaAmountSchema,
  quotaResetSchema,
  type ProviderQuota,
  type ProviderQuotaProbeResult,
  type ProviderQuotaWindow,
} from '@demicodes/provider'
import {
  resolveClaudeCodeOAuthAccess,
  type ClaudeCodeOAuthAccess
} from './oauth'

export interface ClaudeCodeQuotaOptions {
  providerId?: string
  /** Override token resolution (tests / custom stores). */
  resolveAccess?: () => Promise<ClaudeCodeOAuthAccess | null>
  fetch?: (
    input: string | URL | Request,
    init?: RequestInit
  ) => Promise<Response>
  usageUrl?: string
}

const DEFAULT_USAGE_URL = 'https://api.anthropic.com/api/oauth/usage'
const DEFAULT_OAUTH_BETA = 'oauth-2025-04-20'

/**
 * Active probe: GET /api/oauth/usage (Claude.ai consumer plan windows).
 * Observation: anthropic-ratelimit-unified-* response headers.
 */
export function createClaudeCodeQuota(
  options: ClaudeCodeQuotaOptions = {}
): ProviderQuota {
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
      const response = await fetchImpl(
        usageUrl,
        { method: 'GET', headers, signal }
      )
      if (!response.ok) {
        await response.body?.cancel().catch(() => {
          // An errored response body is already closed.
        })
        throw new Error(
          `Claude usage request failed (HTTP ${response.status})`
        )
      }
      const payload = parseProviderJson(claudeQuotaPayloadSchema, await response.text(), 'Claude quota')
      return mapClaudeQuota(payload, access)
    },
    observe: ({ headers, body }) => {
      if (headers) {
        const fromHeaders = observeClaudeRateLimitHeaders(headers)
        if (fromHeaders)
          return fromHeaders
      }
      if (body !== undefined)
        return observeClaudeStreamBody(body)
      return null
    },
  })
}

/** Claude CLI stream-json / status envelopes that embed `rate_limits`. */
export function observeClaudeStreamBody(
  body: unknown
): ProviderQuotaProbeResult | null {
  const envelope = parseProviderData(claudeQuotaEnvelopeSchema, body, 'Claude quota envelope')
  const rateLimits = envelope.rate_limits ?? envelope.message?.rate_limits
  if (!rateLimits) {
    return null
  }
  const partial = mapClaudeQuota(rateLimits)
  return partial.windows.length > 0 ? partial : null
}

export function mapClaudeUsagePayload(
  payload: unknown,
  access?: ClaudeCodeOAuthAccess | null,
): ProviderQuotaProbeResult {
  return mapClaudeQuota(parseProviderData(claudeQuotaPayloadSchema, payload, 'Claude quota'), access)
}

function mapClaudeQuota(
  record: ClaudeQuotaPayload,
  access?: ClaudeCodeOAuthAccess | null,
): ProviderQuotaProbeResult {
  const windows: ProviderQuotaWindow[] = []

  pushWindow(windows, 'five_hour', '5h session', record.five_hour)
  pushWindow(windows, 'seven_day', '7d all models', record.seven_day)
  pushWindow(windows, 'seven_day_sonnet', '7d Sonnet', record.seven_day_sonnet)
  pushWindow(windows, 'seven_day_opus', '7d Opus', record.seven_day_opus)

  if (record.limits) {
    for (const item of record.limits) {
      const kind = item.kind
      // A dedicated window takes precedence only when it is supplied.
      if ((kind === 'session' && record.five_hour)
        || (kind === 'weekly_all' && record.seven_day)) {
        continue
      }
      const percent = clampUsedPercent(item.percent)
      const scopeLabel = item.scope?.model?.display_name
      windows.push({
        id: `limit:${kind}${scopeLabel ? `:${scopeLabel}` : ''}`,
        label: scopeLabel ? `${kind} (${scopeLabel})` : kind,
        usedPercent: percent,
        unit: 'percent',
        resetsAt: item.resets_at ?? null,
        severity: item.severity ?? severityFromUsedPercent(percent),
        scope: scopeLabel ? { kind: 'model', label: scopeLabel } : { kind },
      })
    }
  }

  const planId = access?.subscriptionType ?? null
  return {
    plan: planId ? {
      id: planId,
      label: planId,
      raw: access?.rateLimitTier ?? planId
    } : null,
    accountLabel: null,
    windows,
    raw: record,
  }
}

/** Map anthropic-ratelimit-unified-* headers into a coarse snapshot. */
export function observeClaudeRateLimitHeaders(
  headers: Headers | undefined
): ProviderQuotaProbeResult | null {
  if (!headers)
    return null
  const status = headers.get('anthropic-ratelimit-unified-status')
  const reset = headers.get('anthropic-ratelimit-unified-reset')
  const claim = headers.get('anthropic-ratelimit-unified-representative-claim')
  const overageUtil = headers.get(
    'anthropic-ratelimit-unified-overage-period-channel-utilization'
  )
  if (status === null && reset === null && claim === null && overageUtil === null)
    return null

  const utilization = parseProviderData(quotaAmountSchema.nullable(), numberHeader(
    headers,
    'anthropic-ratelimit-unified-overage-period-channel-utilization',
  ), 'Claude quota utilization')
  // This header describes an overage channel, not a documented quota percentage.
  // Preserve the validated value in raw metadata without guessing its scale.
  const resetsAt = parseProviderData(quotaResetSchema.nullable(), reset, 'Claude quota reset')
  const windows: ProviderQuotaWindow[] = [
    {
      id: 'unified',
      label: claim ?? 'Unified rate limit',
      usedPercent: null,
      unit: 'percent',
      resetsAt,
      severity: status === 'rejected' ? 'critical'
        : status === 'allowed_warning' ? 'warning' : null,
    },
  ]
  return { windows, raw: { status, reset, claim, overageUtil: utilization } }
}

function pushWindow(
  windows: ProviderQuotaWindow[],
  id: string,
  label: string,
  value: ClaudeQuotaWindow | null | undefined,
): void {
  if (!value) {
    return
  }
  const usedPercent = clampUsedPercent(value.utilization ?? value.used_percentage)
  windows.push({
    id,
    label,
    usedPercent,
    unit: 'percent',
    resetsAt: value.resets_at ?? null,
    severity: severityFromUsedPercent(usedPercent),
  })
}

