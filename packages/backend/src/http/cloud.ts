import { Hono } from 'hono'
import { z } from 'zod'
import type { AuthEnv } from '../auth/identity'
import { ManagedHostError, type ManagedHosts } from '../managed/lifecycle'

const resetSchema = z.object({ operationId: z.string().min(1).max(128) }).strict()

export function cloudRoutes(managed: ManagedHosts | null): Hono<AuthEnv> {
  const app = new Hono<AuthEnv>()
  app.get('/', async c => {
    if (!managed)
      return c.json({
        code: 'no_cloud',
        message: 'Cloud is not configured'
      }, 409)
    const status = await managed.status(c.get('user').id)
    return c.json({
      ...status,
      device: status.device
        ? { id: status.device.id, name: status.device.name }
        : null
    })
  })
  app.post('/reset', async c => {
    if (!managed)
      return c.json({
        code: 'no_cloud',
        message: 'Cloud is not configured'
      }, 409)
    const body = resetSchema.safeParse(await c.req.json().catch(() => null))
    if (!body.success)
      return c.json({
        code: 'invalid_body',
        message: 'Expected an operationId'
      }, 400)
    try {
      return c.json(
        { operation: await managed.reset(
            c.get('user').id,
            body.data.operationId
          ) },
        202
      )
    }
    catch (error) {
      if (error instanceof ManagedHostError)
        return c.json({ code: error.code, message: error.message }, 409)
      throw error
    }
  })
  return app
}
