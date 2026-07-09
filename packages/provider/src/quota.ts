/**
 * Unified subscription / rate-limit quota surface for concrete providers.
 *
 * - One snapshot shape for all vendors (windows + optional plan).
 * - Two fill paths: active {@link ProviderQuota.probe} and passive
 *   {@link ProviderQuota.observeResponse} (headers / envelopes).
 * - Products read {@link ProviderQuota.latest} or call {@link ensureQuota}.
 */

export type ProviderQuotaWindowUnit =
  | 'percent'
  | 'credits'
  | 'usd_minor'
  | 'requests'
  | 'tokens'
  | 'unknown'

export type ProviderQuotaSeverity = 'normal' | 'warning' | 'critical'

export interface ProviderQuotaWindow {
  /** Stable id: five_hour, seven_day, primary, monthly, rpm, … */
  id: string
  label?: string
  /** 0–100 when known. */
  usedPercent: number | null
  used?: number | null
  limit?: number | null
  unit?: ProviderQuotaWindowUnit
  /** ISO-8601 reset time when known. */
  resetsAt: string | null
  severity?: ProviderQuotaSeverity | null
  scope?: { kind: string; label?: string } | null
}

export interface ProviderQuotaPlan {
  id: string | null
  label?: string | null
  raw?: string | null
}

export type ProviderQuotaSource = 'probe' | 'observation' | 'cache'

export interface ProviderQuotaSnapshot {
  providerId: string
  observedAt: string
  source: ProviderQuotaSource
  plan?: ProviderQuotaPlan | null
  accountLabel?: string | null
  windows: ProviderQuotaWindow[]
  /** Vendor payload for debugging; not for generic UI logic. */
  raw?: unknown
}

export type ProviderQuotaProbeCost = 'free' | 'minimal_request'

export type ProviderQuotaCapability =
  | { mode: 'none' }
  | {
      mode: 'supported'
      canProbe: boolean
      canObserve: boolean
      /** free = dedicated usage API; minimal_request = burns a tiny inference */
      probeCost?: ProviderQuotaProbeCost
      staleAfterMs?: number
    }

export interface ProviderQuotaProbeOptions {
  signal?: AbortSignal
  /** Bypass in-memory cache freshness for probe implementations that short-circuit. */
  force?: boolean
}

export interface ProviderQuotaObserveInput {
  /** HTTP response headers when the vendor exposes quota there. */
  headers?: Headers
  status?: number
  /**
   * Vendor-specific envelope (e.g. Claude stream-json message with `rate_limits`).
   * Used when there is no raw HTTP Response (CLI transport).
   */
  body?: unknown
}

export interface ProviderQuota {
  capability(): ProviderQuotaCapability
  probe(options?: ProviderQuotaProbeOptions): Promise<ProviderQuotaSnapshot>
  latest(): ProviderQuotaSnapshot | null
  /**
   * Drop the in-memory latest snapshot (e.g. after credentials.setActive so the
   * next ensureQuota/probe does not show the previous account).
   */
  clearLatest?(): void
  /**
   * Optional passive update from a vendor HTTP response (headers, etc.).
   * Returns the new snapshot when observation succeeded; otherwise null.
   */
  observeResponse?(input: ProviderQuotaObserveInput): ProviderQuotaSnapshot | null
}

export class ProviderQuotaUnsupportedError extends Error {
  constructor(readonly providerId: string, message?: string) {
    super(message ?? `Provider "${providerId}" does not support quota probe`)
    this.name = 'ProviderQuotaUnsupportedError'
  }
}

export interface EnsureQuotaOptions {
  /** Prefer a network probe even when cache is fresh. */
  prefer?: 'probe' | 'cache'
  /** Override capability.staleAfterMs when deciding cache freshness. */
  maxStaleMs?: number
  signal?: AbortSignal
}

/**
 * Return a fresh-enough snapshot: cache when valid, else probe when allowed.
 * When only observation is supported and nothing is cached yet, returns null.
 */
export async function ensureQuota(
  quota: ProviderQuota,
  options: EnsureQuotaOptions = {},
): Promise<ProviderQuotaSnapshot | null> {
  const cap = quota.capability()
  if (cap.mode === 'none') return null

  const latest = quota.latest()
  const maxStale = options.maxStaleMs ?? cap.staleAfterMs ?? 60_000
  const fresh =
    latest !== null && Number.isFinite(Date.parse(latest.observedAt))
      ? Date.now() - Date.parse(latest.observedAt) < maxStale
      : false

  if (fresh && latest && options.prefer !== 'probe') {
    return { ...latest, source: 'cache' as const }
  }

  if (cap.canProbe) {
    return quota.probe({ signal: options.signal, force: options.prefer === 'probe' })
  }

  return latest
}

export interface CreateProviderQuotaOptions {
  providerId: string
  canProbe: boolean
  canObserve?: boolean
  probeCost?: ProviderQuotaProbeCost
  staleAfterMs?: number
  /**
   * Active fetch. Should return plan/windows/account/raw; controller fills
   * providerId, observedAt, source.
   */
  probe: (options: ProviderQuotaProbeOptions) => Promise<ProviderQuotaProbeResult>
  /**
   * Optional passive header/body observation. Return null when headers do not
   * carry quota windows.
   */
  observe?: (input: ProviderQuotaObserveInput) => ProviderQuotaProbeResult | null
}

export interface ProviderQuotaProbeResult {
  plan?: ProviderQuotaPlan | null
  accountLabel?: string | null
  windows: ProviderQuotaWindow[]
  raw?: unknown
}

/** In-memory latest snapshot + capability wiring for concrete providers. */
export function createProviderQuota(options: CreateProviderQuotaOptions): ProviderQuota {
  let latest: ProviderQuotaSnapshot | null = null
  const canObserve = options.canObserve ?? Boolean(options.observe)

  const capability = (): ProviderQuotaCapability => {
    if (!options.canProbe && !canObserve) return { mode: 'none' }
    return {
      mode: 'supported',
      canProbe: options.canProbe,
      canObserve,
      probeCost: options.probeCost,
      staleAfterMs: options.staleAfterMs,
    }
  }

  const materialize = (
    partial: ProviderQuotaProbeResult,
    source: Exclude<ProviderQuotaSource, 'cache'>,
  ): ProviderQuotaSnapshot => {
    const snapshot: ProviderQuotaSnapshot = {
      providerId: options.providerId,
      observedAt: new Date().toISOString(),
      source,
      plan: partial.plan ?? null,
      accountLabel: partial.accountLabel ?? null,
      windows: partial.windows,
      raw: partial.raw,
    }
    latest = snapshot
    return snapshot
  }

  const quota: ProviderQuota = {
    capability,
    latest: () => latest,
    clearLatest: () => {
      latest = null
    },
    async probe(probeOptions = {}) {
      if (!options.canProbe) throw new ProviderQuotaUnsupportedError(options.providerId)
      const partial = await options.probe(probeOptions)
      return materialize(partial, 'probe')
    },
  }

  if (options.observe) {
    const observe = options.observe
    quota.observeResponse = (input) => {
      const partial = observe(input)
      if (!partial) return null
      return materialize(partial, 'observation')
    }
  }

  return quota
}

/** Clamp percent into 0–100 or null. */
export function clampUsedPercent(value: unknown): number | null {
  if (typeof value !== 'number' || !Number.isFinite(value)) return null
  if (value < 0) return 0
  if (value > 100) return 100
  return value
}

/** used/limit → percent, or null when limit is missing/zero. */
export function usedPercentFromRatio(used: number | null | undefined, limit: number | null | undefined): number | null {
  if (used == null || limit == null || !Number.isFinite(used) || !Number.isFinite(limit) || limit <= 0) return null
  return clampUsedPercent((used / limit) * 100)
}

export function unixSecondsToIso(value: unknown): string | null {
  if (typeof value === 'number' && Number.isFinite(value)) {
    // Heuristic: ms vs seconds
    const ms = value > 1e12 ? value : value * 1000
    return new Date(ms).toISOString()
  }
  if (typeof value === 'string' && value.trim()) {
    const asNum = Number(value)
    if (Number.isFinite(asNum) && /^\d+(\.\d+)?$/.test(value.trim())) {
      return unixSecondsToIso(asNum)
    }
    const parsed = Date.parse(value)
    if (Number.isFinite(parsed)) return new Date(parsed).toISOString()
  }
  return null
}

export function severityFromUsedPercent(usedPercent: number | null): ProviderQuotaSeverity | null {
  if (usedPercent == null) return null
  if (usedPercent >= 95) return 'critical'
  if (usedPercent >= 80) return 'warning'
  return 'normal'
}
