import type { Host } from '@demicodes/shell'
import { errorMessage } from '@demicodes/utils'
import { Hono, type Context } from 'hono'
import { z } from 'zod'
import type { AuthEnv } from '../auth/identity'
import { conversationSummary } from '../conversation/summary'
import {
  conversationPatchSchema,
  type ConversationUpdates,
} from '../conversation/updates'
import type { AgentServer } from '@demicodes/agent'
import { ATTACHMENT_MAX_BYTES } from './attachments'
import { type ControlService } from '../storage/control'
import type { RunnerRegistry } from '../runner/registry'
import { resolveExecutionTarget } from '../conversation/execution-target'
import type { ConversationStores } from '../storage/conversation-store'
import { ForkRefused, type ConversationForks } from '../conversation/fork'

const attachBodySchema = z.object({ deviceId: z.string().min(1) })
const renameHostBodySchema = z.object({ name: z.string().trim().min(1).max(64) })

/** `/api/conversations` REST surface (the live stream is `stream.ts`). */
export function conversationRoutes(options: {
  admitFrame: (id: string) => (() => void) | null
  updates: ConversationUpdates
  forks: ConversationForks
  agentServer: AgentServer
  control: ControlService
  conversationStores: ConversationStores
  withHost: <T>(
    conversationId: string,
    operation: (host: Host) => Promise<T>,
    signal?: AbortSignal,
  ) => Promise<T>
  /** Whether a device has a live runner socket, for the host list. */
  registry: RunnerRegistry
}): Hono<AuthEnv> {
  const { control, conversationStores, withHost, registry } = options
  const app = new Hono<AuthEnv>()

  // The caller's conversation, or null: another user's answers like a missing one.
  const own = async (c: Context<AuthEnv>) => {
    const conversation = await control.getConversation(c.req.param('id') ?? '')
    return conversation && conversation.userId === c.get('user').id
      ? conversation
      : null
  }

  app.get('/', async (c) => {
    const parsed = z
      .enum(['true', 'false'])
      .optional()
      .safeParse(c.req.query('archived'))
    if (!parsed.success) {
      return c.json(
        {
          code: 'invalid_query',
          message: 'archived must be true or false',
        },
        400,
      )
    }
    const archived = parsed.data === 'true'
    const conversations = await control.listConversations(c.get('user').id, {
      archived,
    })
    return c.json({
      conversations: conversations.map((conversation) =>
        conversationSummary(conversation, conversationStores, options.agentServer),
      ),
    })
  })

  app.post('/', async (c) => {
    const parsed = z.strictObject({ id: z.uuid() }).safeParse(
      await c.req.json().catch(() => null),
    )
    if (!parsed.success) {
      return c.json(
        { code: 'invalid_request', message: 'A conversation UUID is required' },
        400,
      )
    }
    const userId = c.get('user').id
    const existing = await control.getConversation(parsed.data.id)
    if ((existing && existing.userId !== userId) || await control.getConversationFork(parsed.data.id)) {
      return c.json(
        { code: 'id_unavailable', message: 'Conversation id is unavailable' },
        409,
      )
    }
    return c.json(
      { conversation: await control.createConversation(userId, parsed.data) },
      existing ? 200 : 201,
    )
  })

  // Fork copies a snapshot and does not enter source mutation or activity admission.
  app.post('/:id/fork', async (c) => {
    const parsed = z.strictObject({ id: z.uuid(), blockId: z.string().min(1) })
      .safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) {
      return c.json({ code: 'invalid_request', message: 'Expected a destination UUID and assistant blockId' }, 400)
    }
    try {
      const result = await options.forks.create(
        c.get('user').id, c.req.param('id'), parsed.data.id, parsed.data.blockId,
      )
      return c.json({ conversation: conversationSummary(result.conversation, conversationStores, options.agentServer), model: result.model },
        result.created ? 201 : 200)
    } catch (error) {
      if (error instanceof ForkRefused) {
        return c.json({ code: error.code, message: error.message }, error.status)
      }
      throw error
    }
  })

  app.post('/batch', async (c) => {
    const parsed = z
      .strictObject({
        items: z
          .array(
            z.strictObject({
              id: z.string().min(1),
              patch: conversationPatchSchema,
            }),
          )
          .min(1)
          .max(100),
      })
      .safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) {
      return c.json(
        {
          code: 'invalid_body',
          message: 'Expected up to 100 { id, patch } items',
        },
        400,
      )
    }
    const results = []
    for (const item of parsed.data.items) {
      const result = await options.updates.apply(
        c.get('user').id,
        item.id,
        item.patch,
      )
      results.push({
        id: item.id,
        ...(result ?? {
          code: 'conversation_not_found',
          message: 'No such conversation',
        }),
      })
    }
    return c.json({ results }, 207)
  })

  app.patch('/:id', async (c) => {
    const parsed = conversationPatchSchema.safeParse(
      await c.req.json().catch(() => null),
    )
    if (!parsed.success) {
      return c.json(
        {
          code: 'invalid_body',
          message: parsed.error.issues[0]?.message ?? 'Invalid patch',
        },
        400,
      )
    }
    const result = await options.updates.apply(
      c.get('user').id,
      c.req.param('id'),
      parsed.data,
    )
    if (!result) {
      return c.json(
        {
          code: 'conversation_not_found',
          message: 'No such conversation',
        },
        404,
      )
    }
    const failed = result.results.filter((field) => field.status === 'failed')
    if (failed.length === 0) {
      return c.json(result)
    }
    const single = result.results.length === 1 ? failed[0] : undefined
    return c.json(
      {
        ...result,
        ...(single
          ? {
              code: single.code,
              message: single.message,
            }
          : {}),
      },
      single?.httpStatus ?? 207,
    )
  })

  app.post('/:id/read', async (c) => {
    const conversation = await own(c)
    if (!conversation) {
      return c.json(
        {
          code: 'conversation_not_found',
          message: 'No such conversation',
        },
        404,
      )
    }
    const parsed = z
      .strictObject({ revision: z.number().int().nonnegative() })
      .safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) {
      return c.json(
        {
          code: 'invalid_body',
          message: 'Expected { revision }',
        },
        400,
      )
    }
    if (
      parsed.data.revision > conversationStores.summary(conversation.id).revision
    ) {
      return c.json(
        {
          code: 'invalid_revision',
          message: 'Cannot read beyond current output',
        },
        409,
      )
    }
    await control.markConversationRead(conversation.id, parsed.data.revision)
    return c.body(null, 204)
  })

  app.use('/:id/*', async (c, next) => {
    if (c.req.method === 'GET') {
      return next()
    }
    const release = options.admitFrame(c.req.param('id') ?? '')
    if (!release) {
      return c.json(
        {
          code: 'conversation_busy',
          message: 'A conversation operation is still running',
        },
        409,
      )
    }
    try {
      const conversation = await own(c)
      if (conversation?.archived) {
        return c.json(
          {
            code: 'archived',
            message: 'Restore the conversation before editing it',
          },
          409,
        )
      }
      await next()
    } finally {
      release()
    }
  })

  // The attached hosts (`sessions-and-targets.md` § Attached hosts): the
  // hosts this conversation reaches besides its main host. The route takes
  // any device the user owns; which devices the product offers is its own
  // choice. A change is announced to the model at the next turn boundary.
  const hostsOf = async (conversationId: string) =>
    (await control.listAttachedHosts(conversationId)).map((host) => ({
      ...host,
      online: registry.deviceOnline(host.deviceId),
    }))

  app.get('/:id/hosts', async (c) => {
    const conversation = await own(c)
    if (!conversation) {
      return c.json(
        {
          code: 'conversation_not_found',
          message: 'No such conversation',
        },
        404,
      )
    }
    return c.json({ hosts: await hostsOf(conversation.id) })
  })

  app.post('/:id/hosts', async (c) => {
    const conversation = await own(c)
    if (!conversation) {
      return c.json(
        {
          code: 'conversation_not_found',
          message: 'No such conversation',
        },
        404,
      )
    }
    const parsed = attachBodySchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success) {
      return c.json(
        {
          code: 'invalid_body',
          message: 'Expected { deviceId: string }',
        },
        400,
      )
    }
    const device = await control.getDevice(parsed.data.deviceId)
    if (!device || device.userId !== conversation.userId) {
      return c.json(
        {
          code: 'device_not_found',
          message: 'No such device',
        },
        404,
      )
    }
    // A host is main or attached, never both.
    const target = (
      await resolveExecutionTarget(control, options.registry, conversation)
    ).deviceId
    if (target === device.id) {
      return c.json(
        {
          code: 'host_is_main',
          message: "That device is the conversation's main host",
        },
        409,
      )
    }
    await control.attachHost(conversation.id, device.id, device.name, null, true)
    return c.json({ hosts: await hostsOf(conversation.id) }, 201)
  })

  app.patch('/:id/hosts/:deviceId', async (c) => {
    const conversation = await own(c)
    if (!conversation) {
      return c.json(
        {
          code: 'conversation_not_found',
          message: 'No such conversation',
        },
        404,
      )
    }
    const parsed = renameHostBodySchema.safeParse(
      await c.req.json().catch(() => null),
    )
    if (!parsed.success) {
      return c.json(
        {
          code: 'invalid_body',
          message: 'Expected { name: string }',
        },
        400,
      )
    }
    switch (
      await control.renameAttachedHost(
        conversation.id,
        c.req.param('deviceId'),
        parsed.data.name,
      )
    ) {
      case 'not_attached':
        return c.json(
          {
            code: 'host_not_attached',
            message: 'No such attached host',
          },
          404,
        )
      case 'name_taken':
        return c.json(
          {
            code: 'name_taken',
            message: 'Another attached host has that name',
          },
          409,
        )
      case 'renamed':
        return c.json({ hosts: await hostsOf(conversation.id) })
    }
  })

  app.delete('/:id/hosts/:deviceId', async (c) => {
    const conversation = await own(c)
    if (!conversation) {
      return c.json(
        {
          code: 'conversation_not_found',
          message: 'No such conversation',
        },
        404,
      )
    }
    await control.detachHost(conversation.id, c.req.param('deviceId'))
    return c.body(null, 204)
  })

  // Workspace file drop: bytes land in the execution target's working
  // directory over the ordinary Host fs — filesystem data, not conversation
  // data. The returned path is what the client inserts as a text reference.
  app.get('/:id/transcript', async (c) => {
    const conversation = await own(c)
    if (!conversation) {
      return c.json(
        {
          code: 'conversation_not_found',
          message: 'No such conversation',
        },
        404,
      )
    }
    return c.json({
      blocks: conversationStores.transcriptBlocks(conversation.id),
      subagents: await conversationStores.subagentHistory(conversation.id),
    })
  })

  return app
}
