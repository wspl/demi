import { Hono } from 'hono'
import { z } from 'zod'
import { hashPassword } from '../auth/passwords'
import type { WebSessions } from '../auth/sessions'
import type { ControlService } from '../storage/control'
import { writeSessionCookie } from './cookies'
import { passwordSchema, usernameSchema } from './auth'

const setupBodySchema = z.object({ username: usernameSchema, password: passwordSchema })

/**
 * `/api/setup` — the instance's initial setup (`product.md` § User system):
 * creates the master account while the instance has no users, 404 once it
 * has one. The master is signed in by the same response.
 */
export function setupRoutes(options: { control: ControlService; sessions: WebSessions }): Hono {
  const { control, sessions } = options
  const app = new Hono()

  app.get('/', async (c) => c.json({ needed: (await control.countUsers()) === 0 }))

  app.post('/', async (c) => {
    const parsed = setupBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) return c.json({ code: 'invalid_body', message: 'Expected { username, password }' }, 400)
    const user = await control.createMaster({ username: parsed.data.username, passwordHash: await hashPassword(parsed.data.password) })
    if (!user) return c.json({ code: 'already_set_up', message: 'This instance has its master account' }, 404)
    const session = await sessions.open(user.id)
    writeSessionCookie(c, session.token, session.expiresAt)
    return c.json({ user }, 201)
  })

  return app
}
