import { Hono } from 'hono'
import { z } from 'zod'
import type { AuthEnv } from '../auth/identity'
import { ExposeError, type Exposes } from './records'
import type { ExposeRecord } from '../storage/control'
import type { ControlService } from '../storage/control'
import type { RunnerRegistry } from '../runner/registry'

const createBodySchema = z.object({
  deviceId: z.string().min(1),
  address: z.string().min(1)
})

/**
 * `/api/exposes` (`web-api.md` § Exposes): the product surface of the
 * records — list, create, renew, remove. Requests on an expose hostname are
 * not this API; the relay in front of the product routes answers those.
 */
export function exposeRoutes(options: {
  exposes: Exposes
  control: ControlService
  registry: RunnerRegistry
}): Hono<AuthEnv> {
  const { exposes, control, registry } = options
  const app = new Hono<AuthEnv>()
  const view = (record: ExposeRecord) => exposes.view(record)

  app.get('/', async (c) => {
    const records = await exposes.list(c.get('user').id)
    return c.json({ exposes: records.map(view) })
  })

  app.post('/', async (c) => {
    const parsed = createBodySchema.safeParse(
      await c.req.json().catch(() => null)
    )
    if (!parsed.success)
      return c.json({
        code: 'invalid_body',
        message: 'Expected { deviceId: string, address: string }'
      }, 400)
    const device = await control.getDevice(parsed.data.deviceId)
    if (!device || device.userId !== c.get('user').id)
      return c.json({ code: 'device_not_found', message: 'No such device' }, 404)
    if (!registry.deviceOnline(device.id))
      return c.json({
        code: 'device_offline',
        message: 'The device is offline; connect it before exposing a service'
      }, 409)
    try {
      const record = await exposes.add(
        c.get('user').id,
        device,
        parsed.data.address
      )
      return c.json({ expose: view(record) }, 201)
    } catch (error) {
      if (error instanceof ExposeError && error.code === 'expose_unavailable')
        return c.json({ code: error.code, message: error.message }, 409)
      throw error
    }
  })

  app.post('/:id/renew', async (c) => {
    try {
      const record = await exposes.renew(c.get('user').id, c.req.param('id'))
      return c.json({ expose: view(record) })
    } catch (error) {
      if (error instanceof ExposeError)
        return c.json({ code: error.code, message: error.message }, 404)
      throw error
    }
  })

  app.delete('/:id', async (c) => {
    try {
      await exposes.remove(c.get('user').id, c.req.param('id'))
      return c.body(null, 204)
    } catch (error) {
      if (error instanceof ExposeError)
        return c.json({ code: error.code, message: error.message }, 404)
      throw error
    }
  })

  return app
}
