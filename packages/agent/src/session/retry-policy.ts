/**
 * Retry policy for provider requests, applied by the agent runtime (turn loop
 * and compaction) — providers stay single-shot and only classify error codes.
 * A request is retried only while it has produced no transcript content, so a
 * retry can never duplicate partially streamed output.
 */
export interface TurnRetryPolicy {
  /** Total attempts including the first (default 4 = 1 original + 3 retries). */
  maxAttempts: number
  /** Base for exponential backoff with full jitter (default 1000ms). */
  baseDelayMs: number
  /** Backoff ceiling (default 30000ms). */
  maxDelayMs: number
  /** Provider error codes that are retried (default rate_limit, overloaded). */
  retryableCodes: string[]
}

export const DEFAULT_TURN_RETRY_POLICY: TurnRetryPolicy = {
  maxAttempts: 4,
  baseDelayMs: 1_000,
  maxDelayMs: 30_000,
  retryableCodes: ['rate_limit', 'overloaded'],
}

export function resolveRetryPolicy(overrides: Partial<TurnRetryPolicy> | undefined): TurnRetryPolicy {
  return { ...DEFAULT_TURN_RETRY_POLICY, ...overrides }
}

export function isRetryableCode(policy: TurnRetryPolicy, code: string | null): boolean {
  return code !== null && policy.retryableCodes.includes(code)
}

/**
 * Delay before retry `attempt` (1-based): a provider-supplied Retry-After wins,
 * otherwise exponential backoff with full jitter, capped at `maxDelayMs`.
 */
export function retryDelayMs(policy: TurnRetryPolicy, attempt: number, retryAfterMs: number | null): number {
  if (retryAfterMs !== null && Number.isFinite(retryAfterMs) && retryAfterMs >= 0) {
    return Math.min(Math.floor(retryAfterMs), policy.maxDelayMs)
  }
  const ceiling = Math.min(policy.maxDelayMs, policy.baseDelayMs * 2 ** (attempt - 1))
  return Math.floor(Math.random() * ceiling)
}
