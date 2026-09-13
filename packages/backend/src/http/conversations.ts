import { RemoteGitError, type RemoteHost } from '@demicodes/host-remote'
import { errorCode, errorMessage } from '@demicodes/utils'
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
import type { ChangeStore } from '../storage/change-store'
import { ForkRefused, type ConversationForks } from '../conversation/fork'
import { ManagedHostError } from '../managed/lifecycle'
import { TextFileRefused, readTextFile, textOf } from '../runner/file-browser'

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
  changes: ChangeStore
  withHost: <T>(
    conversationId: string,
    operation: (host: RemoteHost) => Promise<T>,
    signal?: AbortSignal,
  ) => Promise<T>
  /** Whether a device has a live runner socket, for the host list. */
  registry: RunnerRegistry
}): Hono<AuthEnv> {
  const { control, conversationStores, withHost, registry } = options
  const summaryDeps = { stores: conversationStores, server: options.agentServer, control, registry }
  const app = new Hono<AuthEnv>()

  // The caller's conversation, or null: another user's answers like a missing one.
  const own = async (c: Context<AuthEnv>) => {
    const conversation = await control.getConversation(c.req.param('id') ?? '')
    return conversation && conversation.userId === c.get('user').id
      ? conversation
      : null
  }

  app.get('/:id/commands/:commandId/changes/file', async (c) => {
    const conversation = await own(c)
    if (!conversation) {
      return c.json({ code: 'not_found', message: 'No such conversation' }, 404)
    }
    const query = z.object({
      path: z.string().min(1),
      edit: z.string().regex(/^(0|[1-9][0-9]*)$/).transform(Number).pipe(z.number().int().nonnegative()),
    }).safeParse(c.req.query())
    if (!query.success) {
      return c.json({ code: 'invalid_query', message: 'Expected a file path and edit index' }, 400)
    }
    const command = c.req.param('commandId')
    const files = conversationStores.commandFiles(conversation.id, command)
    const sides = files && await options.changes.read(conversation.id, command, files, query.data.path, query.data.edit)
    if (!sides) {
      return c.json({ code: 'not_found', message: 'No retained edit' }, 404)
    }
    return c.json(sides)
  })

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
      conversations: await Promise.all(conversations.map((conversation) =>
        conversationSummary(conversation, summaryDeps),
      )),
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
      return c.json({ conversation: await conversationSummary(result.conversation, summaryDeps), model: result.model },
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
  // The working tree behind the change view (`web-api.md` § File text and
  // working tree changes): what is uncommitted, and one file's two sides.
  // Both reach the host through `withHost`, like every other host operation.
  const withWorkingTree = async (
    c: Context<AuthEnv>,
    operation: (host: RemoteHost, root: string) => Promise<Response>,
  ): Promise<Response> => {
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
    try {
      return await withHost(conversation.id, (host) => operation(host, host.defaultCwd))
    } catch (error) {
      return workingTreeError(c, error)
    }
  }

  app.get('/:id/changes', (c) =>
    withWorkingTree(c, async (host, root) => {
      const changes = await host.git.changes(root)
      return c.json({ root, ...changes })
    }),
  )

  app.get('/:id/changes/file', (c) =>
    withWorkingTree(c, async (host, root) => {
      const path = c.req.query('path')
      if (!path || path.startsWith('/') || path.split('/').includes('..')) {
        return c.json(
          {
            code: 'invalid_body',
            message: 'Expected a relative path query parameter',
          },
          400,
        )
      }
      const original = await host.git.show(root, path).then(textOf, (error: unknown) => {
        if (error instanceof RemoteGitError && error.code === 'ENOENT')
          return ''
        throw error
      })
      const modified = await readTextFile(host.fs, `${root}/${path}`).catch((error: unknown) => {
        if (errorCode(error) === 'ENOENT')
          return ''
        throw error
      })
      return c.json({ original, modified })
    }),
  )

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

function workingTreeError(c: Context<AuthEnv>, error: unknown): Response {
  if (errorCode(error) === 'ERUNNEROFFLINE')
    return c.json({ code: 'device_offline', message: 'The execution device is offline' }, 409)
  if (error instanceof ManagedHostError)
    return c.json({ code: error.code, message: error.message }, 503)
  if (error instanceof TextFileRefused)
    return c.json({ code: error.code, message: error.message }, error.code === 'file_too_large' ? 413 : 415)
  if (error instanceof RemoteGitError) {
    switch (error.code) {
      case 'busy':
        return c.json({ code: 'changes_busy', message: error.message }, 503)
      case 'timeout':
        return c.json({ code: 'changes_timeout', message: error.message }, 504)
      case 'not_repository':
        return c.json({ code: 'not_repository', message: error.message }, 409)
      case 'too_large':
        return c.json({ code: 'file_too_large', message: error.message }, 413)
      default:
        return c.json({ code: 'changes_failed', message: error.message }, 500)
    }
  }
  return c.json({ code: 'changes_failed', message: errorMessage(error) }, 500)
}
