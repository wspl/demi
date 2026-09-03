import type { AgentServer } from '@demicodes/agent'
import type { Host } from '@demicodes/shell'
import { errorMessage } from '@demicodes/utils'
import { Hono, type Context } from 'hono'
import { z } from 'zod'
import type { AuthEnv, InstanceMode } from '../auth/identity'
import type { ProviderVault } from '../vault/providers'
import { providerOwner } from '../vault/scope'
import { HOSTLESS_HOME } from '../conversation/scoped-transport'
import { switchConversationTarget } from '../conversation/target-switch'
import { ATTACHMENT_MAX_BYTES } from './attachments'
import type { ControlService } from '../storage/control'
import type { ConversationStores } from '../storage/conversation-store'
import type { ManagedHosts } from '../managed/lifecycle'

const grantBodySchema = z.object({ deviceId: z.string().min(1) })

const patchConversationBodySchema = z.object({
  title: z.string().optional(),
  archived: z.boolean().optional(),
  providerId: z.string().nullable().optional(),
  modelId: z.string().nullable().optional(),
  workspaceId: z.string().nullable().optional(),
})

/** `/api/conversations` REST surface (the live stream is `stream.ts`). */
export function conversationRoutes(options: {
  control: ControlService
  conversationStores: ConversationStores
  agentServer: AgentServer
  hostFor: (conversationId: string) => Promise<Host>
  managedHosts: ManagedHosts | null
  vault: ProviderVault
  mode: InstanceMode
}): Hono<AuthEnv> {
  const { control, conversationStores, agentServer, hostFor, managedHosts, vault, mode } = options
  const app = new Hono<AuthEnv>()

  // The caller's conversation, or null: another user's answers like a missing one.
  const own = async (c: Context<AuthEnv>) => {
    const conversation = await control.getConversation(c.req.param('id') ?? '')
    return conversation && conversation.userId === c.get('user').id ? conversation : null
  }

  app.get('/', async (c) => {
    const archived = c.req.query('archived') === 'true'
    return c.json({ conversations: await control.listConversations(c.get('user').id, { archived }) })
  })

  app.post('/', async (c) => c.json({ conversation: await control.createConversation(c.get('user').id) }, 201))

  app.patch('/:id', async (c) => {
    const conversation = await own(c)
    if (!conversation) return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
    const parsed = patchConversationBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) {
      const issue = parsed.error.issues[0]
      return c.json(
        { code: 'invalid_body', message: `Invalid request body${issue ? `: ${issue.path.join('.')} ${issue.message}` : ''}` },
        400,
      )
    }
    const body = parsed.data
    if (body.title !== undefined && body.title.trim()) {
      await control.renameConversation(conversation.id, body.title.trim())
    }
    if (body.archived !== undefined) {
      await control.setConversationArchived(conversation.id, body.archived)
      // An archived owner's guest is destroyed; its home stays (`managed-hosts.md` § Lifecycle).
      if (body.archived) await managedHosts?.destroy({ kind: 'conversation', id: conversation.id })
    }
    if (body.providerId !== undefined || body.modelId !== undefined) {
      if (body.providerId) {
        const provider = await vault.get(body.providerId)
        if (!provider || provider.ownerUserId !== providerOwner(mode, conversation.userId)) {
          return c.json({ code: 'provider_not_found', message: 'No such provider' }, 404)
        }
      }
      await control.setConversationModel(conversation.id, body.providerId ?? null, body.modelId ?? null)
    }
    if (body.workspaceId !== undefined) {
      const result = await switchConversationTarget({ control, agentServer }, conversation.id, body.workspaceId)
      switch (result.outcome) {
        case 'switched':
        case 'noop':
          break
        case 'workspace_not_found':
          return c.json({ code: 'workspace_not_found', message: 'No such workspace' }, 404)
        case 'no_hostless_entrance':
          return c.json({ code: 'no_hostless_entrance', message: 'A conversation with a machine of its own cannot go back to hostless' }, 409)
        case 'turn_in_flight':
          return c.json({ code: 'turn_in_flight', message: 'Target switches happen at turn boundaries; a turn is running' }, 409)
        case 'conflict':
          return c.json({ code: 'switch_conflict', message: 'A concurrent switch won; re-read the conversation' }, 409)
        case 'conversation_not_found':
          return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
      }
    }
    return c.json({ conversation: await control.getConversation(conversation.id) })
  })

  // The grant set (`sessions-and-targets.md` § Host grants): the hosts this
  // conversation may reach besides its target. Explicit grants take the
  // user's own paired devices; a managed host enters only by the automatic
  // grant on switching away from it.
  app.get('/:id/grants', async (c) => {
    const conversation = await own(c)
    if (!conversation) return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
    return c.json({ grants: await control.listHostGrants(conversation.id) })
  })

  app.post('/:id/grants', async (c) => {
    const conversation = await own(c)
    if (!conversation) return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
    const parsed = grantBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) return c.json({ code: 'invalid_body', message: 'Expected { deviceId: string }' }, 400)
    const device = await control.getDevice(parsed.data.deviceId)
    if (!device || device.userId !== conversation.userId || device.kind !== 'user') {
      return c.json({ code: 'device_not_found', message: 'No such device' }, 404)
    }
    await control.grantHost(conversation.id, device.id)
    return c.json({ grants: await control.listHostGrants(conversation.id) }, 201)
  })

  app.delete('/:id/grants/:deviceId', async (c) => {
    const conversation = await own(c)
    if (!conversation) return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
    await control.revokeHost(conversation.id, c.req.param('deviceId'))
    return c.body(null, 204)
  })

  // Workspace file drop: bytes land in the execution target's working
  // directory over the ordinary Host fs — filesystem data, not conversation
  // data. The returned path is what the client inserts as a text reference.
  app.post('/:id/workspace-files', async (c) => {
    const conversation = await own(c)
    if (!conversation) return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
    const name = c.req.query('name')
    if (!name || !isSafeRelativePath(name)) {
      return c.json({ code: 'invalid_body', message: 'Provide ?name= as a relative file path' }, 400)
    }
    const bytes = new Uint8Array(await c.req.arrayBuffer())
    if (bytes.length === 0) return c.json({ code: 'invalid_body', message: 'Empty upload' }, 400)
    if (bytes.length > ATTACHMENT_MAX_BYTES) {
      return c.json({ code: 'too_large', message: `File exceeds the ${ATTACHMENT_MAX_BYTES}-byte limit` }, 413)
    }
    const workspace = conversation.workspaceId ? await control.getWorkspace(conversation.workspaceId) : null
    const cwd = workspace?.path ?? HOSTLESS_HOME
    try {
      const host = await hostFor(conversation.id)
      await host.fs.writeFile(name, bytes, { cwd, createParents: true })
    } catch (error) {
      return c.json({ code: 'write_failed', message: errorMessage(error) }, 409)
    }
    return c.json({ path: `${cwd.replace(/\/+$/, '')}/${name}` }, 201)
  })

  app.get('/:id/transcript', async (c) => {
    const conversation = await own(c)
    if (!conversation) return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
    return c.json({ blocks: conversationStores.transcriptBlocks(conversation.id) })
  })

  return app
}

function isSafeRelativePath(name: string): boolean {
  if (name.includes('\0') || name.startsWith('/')) return false
  const segments = name.split('/')
  return segments.every((segment) => segment !== '' && segment !== '.' && segment !== '..')
}
