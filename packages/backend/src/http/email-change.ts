import { Hono } from 'hono'
import { z } from 'zod'
import type { AuthEnv } from '../auth/identity'
import type { EmailChanges } from '../auth/email-change'
import type { ControlService } from '../storage/control'
import { emailSchema, passwordSchema } from '@demicodes/product-contracts'

const startSchema = z.strictObject(
  { email: emailSchema, password: passwordSchema }
)
const confirmSchema = z.strictObject({
  id: z.string().min(1),
  code: z.string().regex(/^\d{6}$/)
})

/**
 * A current password starts a challenge; only a code delivered to the new
 * address changes login identity.
 */
export function emailChangeRoutes(
  changes: EmailChanges,
  control: ControlService
): Hono<AuthEnv> {
  const app = new Hono<AuthEnv>()
  app.post('/', async (c) => {
    const parsed = startSchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success)
      return c.json({
        code: 'invalid_body',
        message: 'Expected { email, password }'
      }, 400)
    const result = await changes.start(
      c.get('user').id,
      parsed.data.email,
      parsed.data.password
    )
    if (result.code) {
      const status = result.code === 'invalid_credentials' ? 401
      : result.code === 'email_taken' ? 409
      : result.code === 'too_many_attempts' ? 429 : 503
      return c.json({
        code: result.code,
        message: messages[result.code]
      }, status)
    }
    return c.json({ challenge: result }, 202)
  })
  app.post('/confirm', async (c) => {
    const parsed = confirmSchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success)
      return c.json({
        code: 'invalid_body',
        message: 'Expected { id, code } with six digits'
      }, 400)
    const userId = c.get('user').id
    const outcome = await changes.confirm(
      userId,
      parsed.data.id,
      parsed.data.code
    )
    if (outcome === 'invalid_code')
      return c.json({
        code: outcome,
        message: 'Invalid or expired verification code'
      }, 400)
    if (outcome === 'email_taken')
      return c.json({ code: outcome, message: messages.email_taken }, 409)
    return c.json({ user: await control.getUser(userId) })
  })
  return app
}

const messages = {
  invalid_credentials: 'Current password is wrong',
  email_taken: 'That email is already in use',
  too_many_attempts: 'Wait a minute before requesting another code',
  mail_unavailable: 'Email delivery is not configured',
  mail_failed: 'Verification email could not be delivered; try again',
} as const
