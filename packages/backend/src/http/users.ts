import { Hono } from 'hono'
import { z } from 'zod'
import { outranks, type AuthEnv } from '../auth/identity'
import { hashPassword } from '../auth/passwords'
import type { ControlService } from '../storage/control'
import { passwordSchema, usernameSchema } from './auth'
import { requireAdmin } from './authenticate'

const createUserBodySchema = z.object({ username: usernameSchema, password: passwordSchema, role: z.enum(['admin', 'user']) })
const patchUserBodySchema = z.object({ password: passwordSchema })

/**
 * `/api/users` — account management (`product.md` § User system): admins
 * list accounts, create users and reset their passwords; only the master
 * creates admins or resets an admin's password. A role acts on strictly
 * lower roles, so nobody touches the master. No deletion: no user data is
 * deleted in v1.
 */
export function userRoutes(options: { control: ControlService }): Hono<AuthEnv> {
  const { control } = options
  const app = new Hono<AuthEnv>()

  app.use('*', requireAdmin)

  app.get('/', async (c) => c.json({ users: await control.listUsers() }))

  app.post('/', async (c) => {
    const parsed = createUserBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) return c.json({ code: 'invalid_body', message: 'Expected { username, password (8+ characters), role: admin | user }' }, 400)
    const { username, password, role } = parsed.data
    if (!outranks(c.get('user').role, role)) return c.json({ code: 'forbidden', message: 'Only the master creates admins' }, 403)
    const user = await control.createUser({ username, passwordHash: await hashPassword(password), role })
    if (!user) return c.json({ code: 'username_taken', message: 'That username exists' }, 409)
    return c.json({ user }, 201)
  })

  app.patch('/:id', async (c) => {
    const target = await control.getUser(c.req.param('id') ?? '')
    if (!target) return c.json({ code: 'user_not_found', message: 'No such user' }, 404)
    if (!outranks(c.get('user').role, target.role)) return c.json({ code: 'forbidden', message: 'A role acts on lower roles only' }, 403)
    const parsed = patchUserBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) return c.json({ code: 'invalid_body', message: 'Expected { password } of 8+ characters' }, 400)
    await control.setUserPassword(target.id, await hashPassword(parsed.data.password))
    return c.body(null, 204)
  })

  return app
}
