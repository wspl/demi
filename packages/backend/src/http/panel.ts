import { Hono } from 'hono'
import { z } from 'zod'
import type { AuthEnv } from '../auth/identity'
import type { ControlService } from '../storage/control'

/** The most tabs and bytes of one saved work panel (`web-api.md` § Work panel state). */
const PANEL_TABS_MAX = 64
const PANEL_BYTES_MAX = 64 * 1024

/**
 * The document's shape. `kind` and `data` mean something only to the page:
 * the backend stores them and never interprets them.
 */
const panelSchema = z.strictObject({
  selection: z.string().min(1),
  tabs: z.array(z.strictObject({
    id: z.string().min(1),
    kind: z.string().min(1),
    data: z.unknown(),
  })).max(PANEL_TABS_MAX),
})

const EMPTY_PANEL: z.infer<typeof panelSchema> = { selection: 'change', tabs: [] }

/** `GET/PUT /api/conversations/:id/panel` (`web-api.md` § Work panel state). */
export function panelRoutes(options: { control: ControlService }): Hono<AuthEnv> {
  const { control } = options
  const app = new Hono<AuthEnv>()

  app.get('/:id/panel', async (c) => {
    const conversation = await control.getConversation(c.req.param('id'))
    if (!conversation || conversation.userId !== c.get('user').id)
      return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
    const stored = await control.getConversationPanel(conversation.id)
    if (stored === null)
      return c.json(EMPTY_PANEL)
    // The stored document passed this schema when it was saved; one that no longer does is corrupt, not repaired.
    return c.json(panelSchema.parse(JSON.parse(stored)))
  })

  app.put('/:id/panel', async (c) => {
    const conversation = await control.getConversation(c.req.param('id'))
    if (!conversation || conversation.userId !== c.get('user').id)
      return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
    if (conversation.archived)
      return c.json({ code: 'conversation_archived', message: 'The conversation is archived' }, 409)
    const parsed = panelSchema.safeParse(await c.req.json().catch(() => null))
    if (!parsed.success)
      return c.json({ code: 'invalid_body', message: 'Expected { selection, tabs: [{ id, kind, data }] } with at most 64 tabs' }, 400)
    const document = JSON.stringify(parsed.data)
    if (new TextEncoder().encode(document).length > PANEL_BYTES_MAX)
      return c.json({ code: 'too_large', message: `The work panel is over its ${PANEL_BYTES_MAX}-byte limit` }, 413)
    await control.setConversationPanel(conversation.id, document)
    return c.body(null, 204)
  })

  return app
}
