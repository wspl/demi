import { Hono } from 'hono'
import { z } from 'zod'
import type { AuthEnv } from '../auth/identity'
import type { ControlService } from '../storage/control'

const reorderSchema = z.strictObject({ kind: z.enum(['conversation', 'workspace']), id: z.string().min(1), beforeId: z.string().min(1).nullable() })

export function sidebarRoutes(control: ControlService): Hono<AuthEnv> {
  const app = new Hono<AuthEnv>()
  app.post('/reorder', async (c) => {
    const parsed = reorderSchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) return c.json({ code: 'invalid_body', message: 'Expected { kind, id, beforeId }' }, 400)
    const { kind, id, beforeId } = parsed.data
    const reordered = await control.reorderSidebar(c.get('user').id, kind, id, beforeId)
    if (!reordered) return c.json({ code: 'invalid_order', message: 'Rows must belong to the same project and pin partition' }, 409)
    return c.body(null, 204)
  })
  return app
}
