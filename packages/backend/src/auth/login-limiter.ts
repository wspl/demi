export interface LoginLimiterOptions {
  /** Failures on one username within `lockMs` of each other before it locks. Default 5. */
  lockAfter?: number
  /** How long the lock holds, and how long a name's failures are remembered. Default 60 s. */
  lockMs?: number
  now?: () => number
}

interface Failures {
  count: number
  lockedUntil: number
  /** When the entry is forgotten: `lockMs` after its last failure, which is a lock's end at the latest. */
  expiresAt: number
}

/**
 * Per-username login lockout: `lockAfter` failures within `lockMs` lock the
 * name for `lockMs`; a success clears the count. Entries expire, so a spray
 * of usernames occupies memory for `lockMs` each and no longer.
 */
export class LoginLimiter {
  private readonly lockAfter: number
  private readonly lockMs: number
  private readonly now: () => number
  private readonly failures = new Map<string, Failures>()

  constructor(options: LoginLimiterOptions = {}) {
    this.lockAfter = options.lockAfter ?? 5
    this.lockMs = options.lockMs ?? 60_000
    this.now = options.now ?? Date.now
  }

  locked(username: string): boolean {
    const entry = this.failures.get(username)
    if (!entry) return false
    const now = this.now()
    if (entry.lockedUntil > now) return true
    if (entry.expiresAt <= now) this.failures.delete(username)
    return false
  }

  failed(username: string): void {
    const now = this.now()
    this.forgetExpired(now)
    const entry = this.failures.get(username) ?? { count: 0, lockedUntil: 0, expiresAt: 0 }
    entry.count += 1
    if (entry.count >= this.lockAfter) {
      entry.count = 0
      entry.lockedUntil = now + this.lockMs
    }
    entry.expiresAt = now + this.lockMs
    this.failures.set(username, entry)
  }

  succeeded(username: string): void {
    this.failures.delete(username)
  }

  /** Names tracked right now (diagnostics and tests). */
  get size(): number {
    return this.failures.size
  }

  private forgetExpired(now: number): void {
    for (const [username, entry] of this.failures) {
      if (entry.expiresAt <= now) this.failures.delete(username)
    }
  }
}
