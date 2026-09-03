import { createHash, randomBytes } from 'node:crypto'
import type { ControlService } from '../storage/control'
import type { User } from './identity'

export interface WebSessionsOptions {
  /** How long a session lives from its last renewal. Default 30 days. */
  ttlMs?: number
  /** A request that finds less than this left renews the session to a full ttl. Default 15 days. */
  renewBelowMs?: number
  /** The clock — tests move it. */
  now?: () => number
}

const DAY_MS = 24 * 60 * 60 * 1000

/**
 * Cookie sessions (`product.md` § User system): a 256-bit random token in
 * the browser, its SHA-256 in `web_sessions`, sliding expiry. The token
 * itself is never stored.
 */
export class WebSessions {
  private readonly ttlMs: number
  private readonly renewBelowMs: number
  private readonly now: () => number

  constructor(
    private readonly control: ControlService,
    options: WebSessionsOptions = {},
  ) {
    this.ttlMs = options.ttlMs ?? 30 * DAY_MS
    this.renewBelowMs = options.renewBelowMs ?? 15 * DAY_MS
    this.now = options.now ?? Date.now
  }

  async open(userId: string): Promise<{ token: string; expiresAt: Date }> {
    const token = randomBytes(32).toString('base64url')
    const expiresAt = new Date(this.now() + this.ttlMs)
    await this.control.createWebSession({ tokenHash: hashSessionToken(token), userId, expiresAt: expiresAt.toISOString() })
    return { token, expiresAt }
  }

  /** The session's user, or null when the token is unknown or expired; renews a session near its end. */
  async resolve(token: string): Promise<{ user: User; expiresAt: Date; renewed: boolean } | null> {
    const tokenHash = hashSessionToken(token)
    const session = await this.control.getWebSession(tokenHash)
    if (!session) return null
    const now = this.now()
    let expiresAt = new Date(session.expiresAt)
    if (expiresAt.getTime() <= now) {
      await this.control.deleteWebSession(tokenHash)
      return null
    }
    const user = await this.control.getUser(session.userId)
    if (!user) return null
    let renewed = false
    if (expiresAt.getTime() - now < this.renewBelowMs) {
      expiresAt = new Date(now + this.ttlMs)
      await this.control.extendWebSession(tokenHash, expiresAt.toISOString())
      renewed = true
    }
    return { user, expiresAt, renewed }
  }

  async close(token: string): Promise<void> {
    await this.control.deleteWebSession(hashSessionToken(token))
  }
}

function hashSessionToken(token: string): string {
  return createHash('sha256').update(token).digest('hex')
}
