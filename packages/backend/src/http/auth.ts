import { emailSchema, passwordSchema, nicknameSchema } from '@demicodes/product-contracts'
import { Hono } from 'hono'
import { z } from 'zod'
import type { AuthEnv } from '../auth/identity'
import type { LoginLimiter } from '../auth/login-limiter'
import { hashPassword, verifyPassword } from '../auth/passwords'
import type { WebSessions } from '../auth/sessions'
import type { ControlService } from '../storage/control'
import {
  clearSessionCookie,
  readSessionCookie,
  writeSessionCookie
} from './cookies'

const nicknameBodySchema = z.strictObject(
  { nickname: nicknameSchema }
)
const loginBodySchema = z.strictObject({
  email: emailSchema,
  password: z.string().min(1)
})
const passwordBodySchema = z.strictObject({
  current: z.string().min(1),
  next: passwordSchema
})

/**
 * `/api/auth/*` — login and logout over the session cookie, the caller's
 * identity, the caller's own password.
 */
export function authRoutes(options: {
  control: ControlService;
  sessions: WebSessions;
  limiter: LoginLimiter
}): Hono<AuthEnv> {
  const { control, sessions, limiter } = options
  const app = new Hono<AuthEnv>()

  app.post('/login', async (c) => {
    const parsed = loginBodySchema.safeParse(
      await c.req.json().catch(() => null)
    )
    if (!parsed.success)
      return c.json({
        code: 'invalid_body',
        message: 'Expected { email, password }'
      }, 400)
    const { email, password } = parsed.data
    if (limiter.locked(email))
      return c.json({
        code: 'too_many_attempts',
        message: 'Too many failed logins; try again in a minute'
      }, 429)
    const found = await control.findUserByEmail(email)
    if (!found || !(await verifyPassword(password, found.passwordHash))) {
      limiter.failed(email)
      return c.json({
        code: 'invalid_credentials',
        message: 'Wrong email or password'
      }, 401)
    }
    limiter.succeeded(email)
    const { passwordHash: _hash, ...user } = found
    const session = await sessions.open(user.id)
    writeSessionCookie(c, session.token, session.expiresAt)
    return c.json({ user })
  })

  app.post('/logout', async (c) => {
    const token = readSessionCookie(c)
    if (token)
      await sessions.close(token)
    clearSessionCookie(c)
    return c.body(null, 204)
  })

  app.get('/me', (c) => c.json({ user: c.get('user') }))

  app.patch('/me', async (c) => {
    const parsed = nicknameBodySchema.safeParse(
      await c.req.json().catch(() => null)
    )
    if (!parsed.success)
      return c.json({
        code: 'invalid_body',
        message: 'Expected { nickname }'
      }, 400)
    await control.setUserNickname(c.get('user').id, parsed.data.nickname)
    return c.json({ user: await control.getUser(c.get('user').id) })
  })

  app.put('/password', async (c) => {
    const parsed = passwordBodySchema.safeParse(
      await c.req.json().catch(() => null)
    )
    if (!parsed.success)
      return c.json({
        code: 'invalid_body',
        message: 'Expected { current, next } with at least 8 characters'
      }, 400)
    const user = c.get('user')
    const found = await control.findUserByEmail(user.email)
    if (!found ||
      !(await verifyPassword(parsed.data.current, found.passwordHash))) {
      return c.json({
        code: 'invalid_credentials',
        message: 'Current password is wrong'
      }, 401)
    }
    await control.setUserPassword(user.id, await hashPassword(parsed.data.next))
    return c.body(null, 204)
  })

  return app
}
