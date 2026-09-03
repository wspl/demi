import type { MiddlewareHandler } from 'hono'
import type { AuthEnv } from '../auth/identity'
import type { WebSessions } from '../auth/sessions'
import { clearSessionCookie, readSessionCookie, writeSessionCookie } from './cookies'

/**
 * The session gate over `/api/*`: the cookie names a live session or the
 * request is 401. Paths under `exempt` pass through — the setup and login
 * entrances, and the device-token routes runners dial.
 */
export function authenticate(sessions: WebSessions, exempt: readonly string[]): MiddlewareHandler<AuthEnv> {
  return async (c, next) => {
    const path = c.req.path
    if (exempt.some((prefix) => path === prefix || path.startsWith(`${prefix}/`))) return next()
    const token = readSessionCookie(c)
    const resolved = token ? await sessions.resolve(token) : null
    if (!token || !resolved) {
      if (token) clearSessionCookie(c)
      return c.json({ code: 'unauthenticated', message: 'Sign in first' }, 401)
    }
    if (resolved.renewed) writeSessionCookie(c, token, resolved.expiresAt)
    c.set('user', resolved.user)
    await next()
  }
}

/** The admin gate behind the session gate: master and admin pass, a user is 403. */
export const requireAdmin: MiddlewareHandler<AuthEnv> = async (c, next) => {
  if (c.get('user').role === 'user') return c.json({ code: 'forbidden', message: 'Administrators only' }, 403)
  await next()
}
