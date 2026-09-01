import { Hono } from 'hono'
import { z } from 'zod'
import { STUB_USER } from '../auth/identity'
import type { ControlService } from '../storage/control'
import type { ConversationStores } from '../storage/conversation-store'

const patchConversationBodySchema = z.object({
  title: z.string().optional(),
  archived: z.boolean().optional(),
  connectionId: z.string().nullable().optional(),
  modelId: z.string().nullable().optional(),
})

/** `/api/conversations` REST surface (the live stream is `stream.ts`). */
export function conversationRoutes(options: {
  control: ControlService
  conversationStores: ConversationStores
}): Hono {
  const { control, conversationStores } = options
  const app = new Hono()

  app.get('/', async (c) => {
    const archived = c.req.query('archived') === 'true'
    return c.json({ conversations: await control.listConversations(STUB_USER.id, { archived }) })
  })

  app.post('/', async (c) => c.json({ conversation: await control.createConversation(STUB_USER.id) }, 201))

  app.patch('/:id', async (c) => {
    const conversation = await control.getConversation(c.req.param('id'))
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
    if (body.archived !== undefined) await control.setConversationArchived(conversation.id, body.archived)
    if (body.connectionId !== undefined || body.modelId !== undefined) {
      await control.setConversationModel(conversation.id, body.connectionId ?? null, body.modelId ?? null)
    }
    return c.json({ conversation: await control.getConversation(conversation.id) })
  })

  app.get('/:id/transcript', async (c) => {
    const conversation = await control.getConversation(c.req.param('id'))
    if (!conversation) return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
    return c.json({ blocks: conversationStores.transcriptBlocks(conversation.id) })
  })

  return app
}
