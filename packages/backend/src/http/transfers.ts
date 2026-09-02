import { Hono } from 'hono'
import type { TransferBroker } from '../runner/transfers'
import type { ControlService } from '../storage/control'
import { hashDeviceToken } from '../runner/claim-codes'

/**
 * `/api/transfers/:id` — the two ends of a brokered transfer (`runner.md`
 * § Transfers): the source runner `PUT`s, the destination runner `GET`s,
 * both authenticated by their device token. Never reached by a browser.
 */
export function transferRoutes(options: { control: ControlService; broker: TransferBroker }): Hono {
  const { control, broker } = options
  const app = new Hono()

  const deviceOf = async (authorization: string | undefined): Promise<string | null> => {
    const token = authorization?.match(/^Bearer\s+(\S+)$/i)?.[1]
    if (!token) return null
    return (await control.getDeviceByTokenHash(hashDeviceToken(token)))?.id ?? null
  }

  app.put('/:id', async (c) => {
    const deviceId = await deviceOf(c.req.header('authorization'))
    if (!deviceId) return c.text('device token required', 401)
    const result = await broker.put(c.req.param('id'), deviceId, c.req.raw.body)
    return c.text(result.message, result.status as 200)
  })

  app.get('/:id', async (c) => {
    const deviceId = await deviceOf(c.req.header('authorization'))
    if (!deviceId) return c.text('device token required', 401)
    const result = await broker.get(c.req.param('id'), deviceId)
    if (result.status !== 200 || !('body' in result)) return c.text('message' in result ? result.message : '', result.status as 404)
    return new Response(result.body, { status: 200, headers: { 'content-type': 'application/octet-stream', 'cache-control': 'no-store' } })
  })

  return app
}
