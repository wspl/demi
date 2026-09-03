export interface LoginLimiterOptions {
  /** Failures on one username before it locks. Default 5. */
  lockAfter?: number
  /** How long the lock holds. Default 60 s. */
  lockMs?: number
  now?: () => number
}

/** Per-username login lockout: `lockAfter` failures lock the name for `lockMs`; a success clears the count. */
export class LoginLimiter {
  private readonly lockAfter: number
  private readonly lockMs: number
  private readonly now: () => number
  private readonly failures = new Map<string, { count: number; lockedUntil: number }>()

  constructor(options: LoginLimiterOptions = {}) {
    this.lockAfter = options.lockAfter ?? 5
    this.lockMs = options.lockMs ?? 60_000
    this.now = options.now ?? Date.now
  }

  locked(username: string): boolean {
    const entry = this.failures.get(username)
    if (!entry) return false
    if (entry.lockedUntil > this.now()) return true
    if (entry.lockedUntil > 0) this.failures.delete(username)
    return false
  }

  failed(username: string): void {
    const entry = this.failures.get(username) ?? { count: 0, lockedUntil: 0 }
    entry.count += 1
    if (entry.count >= this.lockAfter) {
      entry.count = 0
      entry.lockedUntil = this.now() + this.lockMs
    }
    this.failures.set(username, entry)
  }

  succeeded(username: string): void {
    this.failures.delete(username)
  }
}
