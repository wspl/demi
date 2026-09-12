import { Hono } from 'hono'
import type { AuthEnv, InstanceMode } from '../auth/identity'
import type { ControlService } from '../storage/control'
import { preferencesPatchSchema } from '@demicodes/product-contracts'

/**
 * Instance mode stays read-only; preferences are isolated by the authenticated
 * user's identity.
 */
export function settingsRoutes(options: {
  mode: InstanceMode;
  control: ControlService
}): Hono<AuthEnv> {
  const app = new Hono<AuthEnv>()
  app.get('/', (c) => c.json({ mode: options.mode }))
  app.get(
    '/preferences',
    async (c) => c.json(
      { preferences: await options.control.getUserPreferences(c.get('user').id) }
    )
  )
  app.patch('/preferences', async (c) => {
    const parsed = preferencesPatchSchema.safeParse(
      await c.req.json().catch(() => null)
    )
    if (!parsed.success)
      return c.json({
        code: 'invalid_body',
        message: 'Expected appearance and/or shortcut overrides'
      }, 400)
    return c.json(
      { preferences: await options.control.patchUserPreferences(
          c.get('user').id,
          parsed.data
        ) }
    )
  })
  return app
}
