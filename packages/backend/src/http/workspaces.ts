import { Hono } from 'hono'
import { z } from 'zod'
import type { AuthEnv } from '../auth/identity'
import type { ControlService, WorkspaceRecord } from '../storage/control'
import { ManagedHostError, type ManagedHosts } from '../managed/lifecycle'

/** A workspace on one of the user's devices at a path, or a Cloud one: a host provisioned for it, the path its home. */
const createWorkspaceBodySchema = z.union([
  z.object({ deviceId: z.string().min(1), path: z.string().min(1), name: z.string().min(1) }),
  z.object({ cloud: z.literal(true), name: z.string().min(1) }),
])

const patchWorkspaceBodySchema = z.object({ name: z.string().min(1) })

/**
 * `/api/workspaces` — CRUD on workspace pointers. A workspace is only a
 * `(device, path, name)` pointer: deleting one removes the pointer and never
 * touches files; deletion is refused while conversations still point at it
 * (moving them is a target switch, with its own turn-boundary rules).
 */
export function workspaceRoutes(options: {
  control: ControlService
  managedHosts: ManagedHosts | null
  /** The Cloud choice; null when this backend provisions no machines. */
  createCloudWorkspace: ((userId: string, name: string) => Promise<WorkspaceRecord>) | null
}): Hono<AuthEnv> {
  const { control, managedHosts, createCloudWorkspace } = options
  const app = new Hono<AuthEnv>()

  app.get('/', async (c) => c.json({ workspaces: await control.listWorkspaces(c.get('user').id) }))

  app.post('/', async (c) => {
    const parsed = createWorkspaceBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) {
      return c.json({ code: 'invalid_body', message: 'Expected { deviceId, path, name } or { cloud: true, name }' }, 400)
    }
    if ('cloud' in parsed.data) {
      if (!createCloudWorkspace) return c.json({ code: 'no_cloud', message: 'This backend provisions no machines' }, 409)
      try {
        return c.json({ workspace: await createCloudWorkspace(c.get('user').id, parsed.data.name) }, 201)
      } catch (error) {
        if (error instanceof ManagedHostError) return c.json({ code: error.code, message: error.message }, 409)
        throw error
      }
    }
    const device = await control.getDevice(parsed.data.deviceId)
    if (!device || device.userId !== c.get('user').id) {
      return c.json({ code: 'device_not_found', message: 'No such device' }, 404)
    }
    const workspace = await control.createWorkspace({ userId: c.get('user').id, ...parsed.data })
    return c.json({ workspace }, 201)
  })

  app.patch('/:id', async (c) => {
    const workspace = await control.getWorkspace(c.req.param('id'))
    if (!workspace || workspace.userId !== c.get('user').id) {
      return c.json({ code: 'workspace_not_found', message: 'No such workspace' }, 404)
    }
    const parsed = patchWorkspaceBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) return c.json({ code: 'invalid_body', message: 'Expected { name: string }' }, 400)
    await control.renameWorkspace(workspace.id, parsed.data.name)
    return c.json({ workspace: await control.getWorkspace(workspace.id) })
  })

  app.delete('/:id', async (c) => {
    const workspace = await control.getWorkspace(c.req.param('id'))
    if (!workspace || workspace.userId !== c.get('user').id) {
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
