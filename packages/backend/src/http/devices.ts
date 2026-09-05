import { errorCode, errorMessage } from '@demicodes/utils'
import { Hono, type Context } from 'hono'
import { z } from 'zod'
import type { AuthEnv } from '../auth/identity'
import type { RunnerRegistry } from '../runner/registry'
import type { ControlService } from '../storage/control'

const claimBodySchema = z.object({ code: z.string().min(1) })
const createDirectoryBodySchema = z.object({ path: z.string().min(1) })

/** `/api/devices` — the device registry surface: list, claim, revoke, directory browse. */
export function deviceRoutes(options: { control: ControlService; registry: RunnerRegistry }): Hono<AuthEnv> {
  const { control, registry } = options
  const app = new Hono<AuthEnv>()

  app.get('/', async (c) => {
    const devices = await control.listDevices(c.get('user').id)
    return c.json({ devices: devices.map((device) => ({ ...device, online: registry.deviceOnline(device.id) })) })
  })

  app.post('/claim', async (c) => {
    const parsed = claimBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) return c.json({ code: 'invalid_body', message: 'Expected { code: string }' }, 400)
    const result = await registry.claim(c.get('user').id, parsed.data.code)
    if (!result.ok) {
      if (result.code === 'rate_limited') return c.json({ code: 'rate_limited', message: 'Too many claim attempts' }, 429)
      return c.json({ code: 'invalid_code', message: 'Unknown or expired pairing code' }, 404)
    }
    return c.json({ device: { ...result.device, online: registry.deviceOnline(result.device.id) } }, 201)
  })

  app.delete('/:id', async (c) => {
    const device = await control.getDevice(c.req.param('id'))
    // A managed host is not one of the user's devices: its row is the lifecycle's and goes with destroy.
    if (!device || device.userId !== c.get('user').id || device.kind !== 'user') {
      return c.json({ code: 'device_not_found', message: 'No such device' }, 404)
    }
    // A workspace is a pointer at a device; revoking under it would dangle it, so the workspace goes first.
    const inUse = await control.countWorkspacesOnDevice(device.id)
    if (inUse > 0) return c.json({ code: 'device_in_use', message: `${inUse} workspace(s) still point at this device` }, 409)
    await registry.revoke(device.id)
    return c.body(null, 204)
  })

  app.get('/:id/fs', async (c) => {
    const outcome = await deviceFsFor(c.req.param('id'), c.get('user').id, control, registry)
    if (!outcome.ok) return c.json(outcome.error, outcome.status)
    const path = c.req.query('path')
    if (!path) return c.json({ code: 'invalid_body', message: 'Missing path query parameter' }, 400)
    try {
      const entries = await outcome.fs.readdir(path, { withFileTypes: true })
      return c.json({
        entries: entries.map((entry) => ({ name: entry.name, isDirectory: entry.isDirectory })),
      })
    } catch (error) {
      return fsError(c, error)
    }
  })

  app.post('/:id/fs', async (c) => {
    const outcome = await deviceFsFor(c.req.param('id'), c.get('user').id, control, registry)
    if (!outcome.ok) return c.json(outcome.error, outcome.status)
    const parsed = createDirectoryBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) return c.json({ code: 'invalid_body', message: 'Expected { path: string }' }, 400)
    try {
      await outcome.fs.mkdir(parsed.data.path, { recursive: true })
      return c.json({ path: parsed.data.path }, 201)
    } catch (error) {
      return fsError(c, error)
    }
  })

  return app
}

async function deviceFsFor(deviceId: string, userId: string, control: ControlService, registry: RunnerRegistry) {
  const device = await control.getDevice(deviceId)
  if (!device || device.userId !== userId) {
    return { ok: false as const, status: 404 as const, error: { code: 'device_not_found', message: 'No such device' } }
  }
  const fs = registry.deviceFs(device.id)
  if (!fs) {
    return { ok: false as const, status: 409 as const, error: { code: 'device_offline', message: 'Device is offline' } }
  }
  return { ok: true as const, fs }
}

function fsError(c: Context, error: unknown) {
  const code = errorCode(error)
  const status = code === 'ENOENT' ? 404 : 400
  return c.json({ code: 'fs_error', message: errorMessage(error) }, status)
}
