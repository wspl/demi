import { Hono } from 'hono'
import { z } from 'zod'
import type { AuthEnv } from '../auth/identity'
import type { LoginLimiter } from '../auth/login-limiter'
import { hashPassword, verifyPassword } from '../auth/passwords'
import type { WebSessions } from '../auth/sessions'
import type { ControlService } from '../storage/control'
import { clearSessionCookie, readSessionCookie, writeSessionCookie } from './cookies'

export const usernameSchema = z.string().trim().min(1).max(64)
export const passwordSchema = z.string().min(8).max(1024)

const loginBodySchema = z.object({ username: usernameSchema, password: z.string().min(1) })
const passwordBodySchema = z.object({ current: z.string().min(1), next: passwordSchema })

/** `/api/auth/*` — login and logout over the session cookie, the caller's identity, the caller's own password. */
export function authRoutes(options: { control: ControlService; sessions: WebSessions; limiter: LoginLimiter }): Hono<AuthEnv> {
  const { control, sessions, limiter } = options
  const app = new Hono<AuthEnv>()

  app.post('/login', async (c) => {
    const parsed = loginBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) return c.json({ code: 'invalid_body', message: 'Expected { username, password }' }, 400)
    const { username, password } = parsed.data
    if (limiter.locked(username)) return c.json({ code: 'too_many_attempts', message: 'Too many failed logins; try again in a minute' }, 429)
    const found = await control.findUserByUsername(username)
    if (!found || !(await verifyPassword(password, found.passwordHash))) {
      limiter.failed(username)
      return c.json({ code: 'invalid_credentials', message: 'Wrong username or password' }, 401)
    }
    limiter.succeeded(username)
    const { passwordHash: _hash, ...user } = found
    const session = await sessions.open(user.id)
    writeSessionCookie(c, session.token, session.expiresAt)
    return c.json({ user })
  })

  app.post('/logout', async (c) => {
    const token = readSessionCookie(c)
    if (token) await sessions.close(token)
    clearSessionCookie(c)
    return c.body(null, 204)
  })

  app.get('/me', (c) => c.json({ user: c.get('user') }))

  app.put('/password', async (c) => {
    const parsed = passwordBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) return c.json({ code: 'invalid_body', message: 'Expected { current, next } with at least 8 characters' }, 400)
    const user = c.get('user')
    const found = await control.findUserByUsername(user.username)
    if (!found || !(await verifyPassword(parsed.data.current, found.passwordHash))) {
      return c.json({ code: 'invalid_credentials', message: 'Current password is wrong' }, 401)
    }
    await control.setUserPassword(user.id, await hashPassword(parsed.data.next))
    return c.body(null, 204)
  })

  return app
}
