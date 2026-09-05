/**
 * Per-user provider-request rate limit — the enforcement half of usage
 * accounting, applied at the inference entry. The limit is a hardcoded
 * generous ceiling (like the virtual-fs quotas): it exists to stop runaway
 * loops and abuse, not to price usage.
 */
export const DEFAULT_PROVIDER_REQUESTS_PER_MINUTE = 120

export class ProviderRateLimiter {
  private readonly windows = new Map<string, number[]>()

  constructor(private readonly requestsPerMinute: number = DEFAULT_PROVIDER_REQUESTS_PER_MINUTE) {}

  /** Records one request; throws a `rate_limited`-coded error when over the ceiling. */
  take(userId: string): void {
    const now = Date.now()
    const window = (this.windows.get(userId) ?? []).filter((at) => now - at < 60_000)
    if (window.length >= this.requestsPerMinute) {
      this.windows.set(userId, window)
      throw Object.assign(new Error(`Provider request rate limit reached (${this.requestsPerMinute}/min)`), {
        code: 'rate_limited',
      })
    }
    window.push(now)
    this.windows.set(userId, window)
  }
}
