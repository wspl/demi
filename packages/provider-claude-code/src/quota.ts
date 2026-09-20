import { z } from 'zod'
import {
  clampUsedPercent,
  createProviderQuota,
  numberHeader,
  severityFromUsedPercent,
  unixSecondsToIso,
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
  /** Where the latest snapshot is kept across rebuilds and restarts (`createProviderQuota`). */
  snapshotFile?: string
}

const DEFAULT_USAGE_URL = 'https://api.anthropic.com/api/oauth/usage'
const DEFAULT_OAUTH_BETA = 'oauth-2025-04-20'

/**
 * The usage payload's fields, as the API and the CLI's `rate_limits`
 * envelopes report them. Quota is a display surface: a malformed window or
 * limit entry is dropped, and the rest of the snapshot still renders.
 */
const resetsAtSchema = z.union([z.string(), z.number()]).optional()
  .catch(undefined)

const usageWindowSchema = z.looseObject({
  utilization: z.number().optional(),
  used_percentage: z.number().optional(),
  resets_at: resetsAtSchema,
}).optional().catch(undefined)

const usageLimitSchema = z.looseObject({
  kind: z.string().min(1),
  percent: z.number().optional(),
  severity: z.enum(['normal', 'warning', 'critical']).optional()
    .catch(undefined),
  resets_at: resetsAtSchema,
  scope: z.looseObject({
    model: z.looseObject({ display_name: z.string().optional() }).optional(),
  }).optional().catch(undefined),
}).optional().catch(undefined)
type ClaudeUsageWindow = NonNullable<z.infer<typeof usageWindowSchema>>

const usagePayloadSchema = z.looseObject({
  five_hour: usageWindowSchema,
  seven_day: usageWindowSchema,
  seven_day_sonnet: usageWindowSchema,
  seven_day_opus: usageWindowSchema,
  limits: z.array(usageLimitSchema).optional().catch(undefined),
})

/** The CLI stream envelopes that embed a usage payload as `rate_limits`. */
const quotaEnvelopeSchema = z.looseObject({
  rate_limits: z.unknown().optional(),
  message: z.looseObject({ rate_limits: z.unknown().optional() })
    .optional().catch(undefined),
})

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
    snapshotFile: options.snapshotFile,
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
        const body = await response.text().catch(() => '')
        throw new Error(
          `Claude usage request failed (${response.status}): ${body.slice(0, 200)}`
        )
      }
      const payload = await response.json()
      return mapClaudeUsagePayload(payload, access)
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
  const envelope = quotaEnvelopeSchema.safeParse(body)
  if (!envelope.success)
    return null
  const rateLimits = envelope.data.rate_limits
    ?? envelope.data.message?.rate_limits
  if (rateLimits === undefined)
    return null
  // Reuse payload mapper shape: five_hour / seven_day on the rate_limits object.
  const partial = mapClaudeUsagePayload(rateLimits)
  return partial.windows.length > 0 ? {
    windows: partial.windows,
    raw: rateLimits
  } : null
}

export function mapClaudeUsagePayload(
  payload: unknown,
  access?: ClaudeCodeOAuthAccess | null,
): ProviderQuotaProbeResult {
  const parsed = usagePayloadSchema.safeParse(payload)
  const record = parsed.success ? parsed.data : {}
  const windows: ProviderQuotaWindow[] = []

  pushWindow(windows, 'five_hour', '5h session', record.five_hour)
  pushWindow(windows, 'seven_day', '7d all models', record.seven_day)
  pushWindow(windows, 'seven_day_sonnet', '7d Sonnet', record.seven_day_sonnet)
  pushWindow(windows, 'seven_day_opus', '7d Opus', record.seven_day_opus)

  for (const item of record.limits ?? []) {
    if (!item)
      continue
    // Prefer dedicated five_hour/seven_day objects when present.
    if (item.kind === 'session' || item.kind === 'weekly_all')
      continue
    const percent = clampUsedPercent(item.percent ?? null)
    const scopeLabel = item.scope?.model?.display_name
    windows.push({
      id: `limit:${item.kind}${scopeLabel ? `:${scopeLabel}` : ''}`,
      label: scopeLabel ? `${item.kind} (${scopeLabel})` : item.kind,
      usedPercent: percent,
      unit: 'percent',
      resetsAt: resetsAtIso(item.resets_at),
      severity: item.severity ?? severityFromUsedPercent(percent),
      scope: scopeLabel
        ? { kind: 'model', label: scopeLabel }
        : { kind: item.kind },
    })
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
    raw: payload,
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
  if (!status && !reset && !claim && !overageUtil)
    return null

  const usedPercent = clampUsedPercent(numberHeader(
    headers,
    'anthropic-ratelimit-unified-overage-period-channel-utilization'
  ))
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
  window: ClaudeUsageWindow | undefined,
): void {
  if (!window)
    return
  const usedPercent = clampUsedPercent(
    window.utilization ?? window.used_percentage ?? null
  )
  windows.push({
    id,
    label,
    usedPercent,
    unit: 'percent',
    resetsAt: resetsAtIso(window.resets_at),
    severity: severityFromUsedPercent(usedPercent),
  })
}

/**
 * A reset time as ISO-8601. The API reports either unix seconds or a date
 * string; a string that is neither is reported back as it arrived.
 */
function resetsAtIso(value: string | number | undefined): string | null {
  return unixSecondsToIso(value) ?? (typeof value === 'string' ? value : null)
}

