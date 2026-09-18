import type { FrameAdmission } from './stream'
import type { ConversationHostAccess, ConversationTargets } from '../conversation/target'
import { previewMediaType } from '@demicodes/core'
import { RemoteGitError, type RemoteHost } from '@demicodes/host-remote'
import type { HostFileStat } from '@demicodes/shell'
import { basenamePath, deferred, errorCode, errorMessage } from '@demicodes/utils'
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
import type { FailureFactsReader } from '../conversation/failure-facts'
import { resolveExecutionTarget } from '../conversation/execution-target'
import type { ConversationStores } from '../storage/conversation-store'
import type { ChangeStore } from '../storage/change-store'
import { ForkRefused, type ConversationForks } from '../conversation/fork'
import { HostAccessRefused } from '../conversation/target'
import { ManagedHostError } from '../managed/lifecycle'
import { TextFileRefused, browseDirectory, readTextFile, textOf } from '../runner/file-browser'
import { booleanQuerySchema } from './query'
import { bunServerOf } from './bun-server'
import { TRANSFER_STALL_MS, contentHeaders, pacedBody, rangeAnswer } from './file-transfer'

const filePathSchema = z.object({ path: z.string().min(1) })
const directoryQuerySchema = filePathSchema.partial()
/** `?path=&version=&download=` of the raw file route. */
const rawFileQuerySchema = filePathSchema.extend({
  version: z.string().min(1).optional(),
  download: booleanQuerySchema,
})

/** `?path=` relative to the working tree: no leading slash, no `..` segment. */
const relativePathQuerySchema = z.object({
  path: z.string().min(1).refine(
    (path) => !path.startsWith('/') && !path.split('/').includes('..'),
    'Expected a relative path',
  ),
})
/** `?path=&download=` of the committed file route. */
const committedFileQuerySchema = relativePathQuerySchema.extend({ download: booleanQuerySchema })
const attachBodySchema = z.object({ deviceId: z.string().min(1) })
const renameHostBodySchema = z.object({ name: z.string().trim().min(1).max(64) })

/**
 * Headers every raw file answer carries (`web-api.md` § File text and
 * working tree changes): revalidated on each use, private to the signed-in
 * user, and streamed rather than buffered by a proxy in front.
 */
const RAW_FILE_HEADERS = {
  'cache-control': 'private, no-cache',
  vary: 'Cookie',
  'x-accel-buffering': 'no',
}

/** A file's version as an ETag: its size and modification time. */
function fileVersion(stat: HostFileStat): string {
  return `W/"${stat.size.toString(16)}-${stat.mtime.getTime().toString(16)}"`
}

/**
 * Whether `If-None-Match` names `etag`, compared weakly (RFC 9110 § 13.1.2).
 * Hono's etag middleware compares the same way but judges a response already
 * made, when a transfer's stream would be open; it exports no comparison.
 */
function notModified(ifNoneMatch: string | undefined, etag: string): boolean {
  const strip = (tag: string) => tag.trim().replace(/^W\//, '')
  return ifNoneMatch !== undefined &&
    (ifNoneMatch.trim() === '*' || ifNoneMatch.split(',').some((tag) => strip(tag) === strip(etag)))
}

/** `/api/conversations` REST surface (the live stream is `stream.ts`). */
export function conversationRoutes(options: {
  admitFrame: FrameAdmission
  updates: ConversationUpdates
  forks: ConversationForks
  agentServer: AgentServer
  control: ControlService
  conversationStores: ConversationStores
  changes: ChangeStore
  withHost: ConversationHostAccess
  /** A file transfer's Host access (`sessions-and-targets.md` § Host operations). */
  transfer: ConversationTargets['transfer']
  /** Whether a device has a live runner socket, for the host list. */
  registry: RunnerRegistry
  /** The failure facts sent beside the root blocks and each subagent history. */
  readFailures: FailureFactsReader
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
    const parsed = booleanQuerySchema.safeParse(c.req.query('archived'))
    if (!parsed.success) {
      return c.json(
        {
          code: 'invalid_query',
          message: 'archived must be true or false',
        },
        400,
      )
    }
    const archived = parsed.data ?? false
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
    // Detach owns exclusive admission and must not first enter as ordinary work.
    if (c.req.method === 'GET' || c.req.method === 'DELETE') {
      return next()
    }
    const release = await options.admitFrame(c.req.param('id') ?? '', c.req.raw.signal)
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
    if (conversation.archived)
      return c.json({ code: 'archived', message: 'Restore the conversation before editing it' }, 409)
    if (!await options.updates.detachHost(conversation.id, c.req.param('deviceId')))
      return c.json({ code: 'turn_in_flight', message: 'Conversation is busy' }, 409)
    return c.body(null, 204)
  })

  // Browser reads and working-tree queries share conversation Host admission.
  const withConversationHost = async (
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
      return await withHost(
        conversation.id,
        (host) => operation(host, host.defaultCwd),
        { signal: c.req.raw.signal, deviceId: c.req.param('deviceId') },
      )
    } catch (error) {
      return hostOperationError(c, error)
    }
  }

  app.on('GET', ['/:id/fs', '/:id/hosts/:deviceId/fs'], async (c) => {
    const query = directoryQuerySchema.safeParse(c.req.query())
    if (!query.success) {
      return c.json({ code: 'invalid_query', message: 'Expected a nonempty path' }, 400)
    }
    return withConversationHost(c, async (host, root) => {
      const path = query.data.path ?? root
      const entries = await browseDirectory(host.fs, path)
      return c.json({ path, home: host.identity.homeDir, entries })
    })
  })

  app.on('POST', ['/:id/fs', '/:id/hosts/:deviceId/fs'], async (c) => {
    const body = filePathSchema.safeParse(await c.req.json().catch(() => null))
    if (!body.success) {
      return c.json({ code: 'invalid_body', message: 'Expected { path: string }' }, 400)
    }
    return withConversationHost(c, async (host) => {
      await host.fs.mkdir(body.data.path, { recursive: true })
      return c.json({ path: body.data.path }, 201)
    })
  })

  app.get('/:id/fs/file', async (c) => {
    const query = filePathSchema.safeParse(c.req.query())
    if (!query.success) {
      return c.json({ code: 'invalid_query', message: 'Expected a path query parameter' }, 400)
    }
    return withConversationHost(c, async (host) => {
      const path = query.data.path
      const text = await readTextFile(host.fs, path)
      return c.json({ path, text })
    })
  })

  app.on(['GET', 'HEAD'], '/:id/fs/raw', async (c) => {
    const query = rawFileQuerySchema.safeParse(c.req.query())
    if (!query.success) {
      return c.json({
        code: 'invalid_query',
        message: 'Expected a path, an optional version, and download as true or false',
      }, 400)
    }
    const conversation = await own(c)
    if (!conversation) {
      return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
    }
    const { path, version, download } = query.data
    // The answer is ready before the transfer ends: a streamed body keeps the
    // transfer, and its Host access, open until the browser has it all.
    const answer = deferred<Response>()
    const transfer = options.transfer(conversation.id, async (host, signal) => {
      const stat = await host.fs.stat(path)
      if (!stat.isFile) {
        answer.resolve(c.json({ code: 'not_found', message: 'Not a regular file' }, 404))
        return
      }
      const etag = fileVersion(stat)
      if (version !== undefined && version !== etag) {
        answer.resolve(c.json({ code: 'file_changed', message: 'The file is no longer the version asked for' }, 412))
        return
      }
      const headers = {
        ...RAW_FILE_HEADERS,
        ...contentHeaders(previewMediaType(path), { download, fileName: basenamePath(path) }),
        etag,
        'last-modified': stat.mtime.toUTCString(),
      }
      if (notModified(c.req.header('if-none-match'), etag)) {
        answer.resolve(new Response(null, { status: 304, headers: { ...RAW_FILE_HEADERS, etag } }))
        return
      }
      const part = rangeAnswer(c.req.header('range'), stat.size)
      if (part.status === 416 || c.req.method === 'HEAD' || part.length === 0) {
        answer.resolve(new Response(null, { status: part.status, headers: { ...headers, ...part.headers } }))
        return
      }
      const bytes = await host.fs.readStream(path, { offset: part.start, length: part.length, signal })
      const server = bunServerOf(c)
      const paced = pacedBody(bytes, {
        stallMs: TRANSFER_STALL_MS,
        signal,
        // The server closes the connection once it has gone idle.
        cutShort: () => server?.timeout(c.req.raw, 1),
      })
      // A connection nothing moves on closes, one whose device stopped
      // sending among them.
      server?.timeout(c.req.raw, TRANSFER_STALL_MS / 1000)
      answer.resolve(new Response(paced.body, { status: part.status, headers: { ...headers, ...part.headers } }))
      await paced.finished
    }, { signal: c.req.raw.signal })
    // A failure before the answer is the answer; one after it cuts the body
    // short instead, and this resolve does nothing.
    transfer.catch((error: unknown) => answer.resolve(hostOperationError(c, error)))
    return answer.promise
  })

  app.get('/:id/changes', (c) =>
    withConversationHost(c, async (host, root) => {
      const changes = await host.git.changes(root)
      return c.json({ root, ...changes })
    }),
  )

  app.get('/:id/changes/file', async (c) => {
    const query = relativePathQuerySchema.safeParse(c.req.query())
    if (!query.success) {
      return c.json({ code: 'invalid_query', message: 'Expected a relative path query parameter' }, 400)
    }
    return withConversationHost(c, async (host, root) => {
      const path = query.data.path
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
    })
  })

  app.on(['GET', 'HEAD'], '/:id/changes/raw', async (c) => {
    const query = committedFileQuerySchema.safeParse(c.req.query())
    if (!query.success) {
      return c.json({ code: 'invalid_query', message: 'Expected a relative path, and download as true or false' }, 400)
    }
    const { path, download } = query.data
    // Git's copy is decoded whole on the runner and is at most 8 MiB, so it
    // is held here and served without a transfer's stream.
    return withConversationHost(c, async (host, root) => {
      const bytes = await host.git.show(root, path)
      const part = rangeAnswer(c.req.header('range'), bytes.byteLength)
      const headers = {
        ...RAW_FILE_HEADERS,
        ...contentHeaders(previewMediaType(path), { download, fileName: basenamePath(path) }),
        ...part.headers,
      }
      if (part.status === 416 || c.req.method === 'HEAD')
        return new Response(null, { status: part.status, headers })
      return new Response(bytes.subarray(part.start, part.start + part.length), { status: part.status, headers })
    })
  })

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
    const blocks = conversationStores.transcriptBlocks(conversation.id)
    const subagents = await conversationStores.subagentHistory(conversation.id)
    return c.json({
      blocks,
      failures: await options.readFailures(blocks),
      subagents: await Promise.all(subagents.map(async (agent) => ({
        ...agent,
        failures: await options.readFailures(agent.blocks),
      }))),
    })
  })

  return app
}

function hostOperationError(c: Context<AuthEnv>, error: unknown): Response {
  if (error instanceof HostAccessRefused) {
    return c.json({ code: error.code, message: error.message }, error.code === 'host_not_attached' ? 404 : 409)
  }
  const code = errorCode(error)
  if (code === 'ENOENT')
    return c.json({ code: 'fs_error', message: errorMessage(error) }, 404)
  if (code === 'EACCES' || code === 'EPERM')
    return c.json({ code: 'fs_error', message: errorMessage(error) }, 403)
  if (code === 'ERUNNEROFFLINE')
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
  // Past git's own, only a directory listing outgrows the runner's message limit.
  if (code === 'too_large')
    return c.json({ code: 'directory_too_large', message: errorMessage(error) }, 413)
  return c.json({ code: 'host_operation_failed', message: errorMessage(error) }, 500)
}
