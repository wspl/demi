import { Hono } from 'hono'
import { z } from 'zod'
import { STUB_USER } from '../auth/identity'
import type { ControlService } from '../storage/control'
import type { ManagedHosts } from '../managed/lifecycle'

const createWorkspaceBodySchema = z.object({
  deviceId: z.string().min(1),
  path: z.string().min(1),
  name: z.string().min(1),
})

const patchWorkspaceBodySchema = z.object({ name: z.string().min(1) })

/**
 * `/api/workspaces` — CRUD on workspace pointers. A workspace is only a
 * `(device, path, name)` pointer: deleting one removes the pointer and never
 * touches files; deletion is refused while conversations still point at it
 * (moving them is a target switch, with its own turn-boundary rules).
 */
export function workspaceRoutes(options: { control: ControlService; managedHosts: ManagedHosts | null }): Hono {
  const { control, managedHosts } = options
  const app = new Hono()

  app.get('/', async (c) => c.json({ workspaces: await control.listWorkspaces(STUB_USER.id) }))

  app.post('/', async (c) => {
    const parsed = createWorkspaceBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) {
      return c.json({ code: 'invalid_body', message: 'Expected { deviceId, path, name }' }, 400)
    }
    const device = await control.getDevice(parsed.data.deviceId)
    if (!device || device.userId !== STUB_USER.id) {
      return c.json({ code: 'device_not_found', message: 'No such device' }, 404)
    }
    const workspace = await control.createWorkspace({ userId: STUB_USER.id, ...parsed.data })
    return c.json({ workspace }, 201)
  })

  app.patch('/:id', async (c) => {
    const workspace = await control.getWorkspace(c.req.param('id'))
    if (!workspace || workspace.userId !== STUB_USER.id) {
      return c.json({ code: 'workspace_not_found', message: 'No such workspace' }, 404)
    }
    const parsed = patchWorkspaceBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) return c.json({ code: 'invalid_body', message: 'Expected { name: string }' }, 400)
    await control.renameWorkspace(workspace.id, parsed.data.name)
    return c.json({ workspace: await control.getWorkspace(workspace.id) })
  })

  app.delete('/:id', async (c) => {
    const workspace = await control.getWorkspace(c.req.param('id'))
    if (!workspace || workspace.userId !== STUB_USER.id) {
      return c.json({ code: 'workspace_not_found', message: 'No such workspace' }, 404)
    }
    const inUse = await control.countConversationsInWorkspace(workspace.id)
    if (inUse > 0) {
      return c.json({ code: 'workspace_in_use', message: `${inUse} conversation(s) still target this workspace` }, 409)
    }
    await managedHosts?.destroy({ kind: 'workspace', id: workspace.id })
    await control.deleteWorkspace(workspace.id)
    return c.body(null, 204)
  })

  return app
}
