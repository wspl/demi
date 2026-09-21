import { RemoteServiceError, RemoteServiceExit, type RemoteHost } from '@demicodes/host-remote'
import { errorCode } from '@demicodes/utils'
import { Hono, type Context } from 'hono'
import { z } from 'zod'
import { browserCreatedBySchema } from '@demicodes/browser-protocol'
import type { AuthEnv } from '../auth/identity'
import { resolveExecutionTarget } from '../conversation/execution-target'
import { HostAccessRefused, type ConversationTargets } from '../conversation/target'
import { buildCommandContext } from '../runner/command-context'
import type { RunnerRegistry } from '../runner/registry'
import type { ControlService, ConversationRecord } from '../storage/control'
import type { UserStreamDeclaration } from './user-streams'

/** The most a browser operation's JSON answer may be. */
const ANSWER_MAX_BYTES = 1024 * 1024

const tabSchema = z.object({
  id: z.string(),
  title: z.string(),
  url: z.string(),
  createdBy: browserCreatedBySchema,
})
/** `browser.tabs`, as its JSON answer lists them. */
const tabsAnswerSchema = z.object({ tabs: z.array(tabSchema) })
/** `browser.open` for a user: the tab, and its address. */
const tabAnswerSchema = z.object({ tab: z.string(), url: z.string().optional() })
/** A failed JSON invocation's standard error. */
const failureSchema = z.object({ error: z.object({ code: z.string(), message: z.string() }) })

interface Reach {
  host: RemoteHost
  cwd: string
}

/**
 * The conversation browser's tab routes (`web-api.md` § Conversation browser
 * tabs). Each runs the operation the agent's command runs, with a `user`
 * caller, as a one-shot user call; the backend holds no browser logic.
 */
export function browserTabRoutes(options: {
  control: ControlService
  registry: Pick<RunnerRegistry, 'deviceIdentity'>
  targets: Pick<ConversationTargets, 'stream' | 'withHost'>
  /** The `browser` user stream's declaration: the package whose operations these are. */
  browser: UserStreamDeclaration
}): Hono<AuthEnv> {
  const { control, registry, targets, browser } = options
  const app = new Hono<AuthEnv>()

  async function own(c: Context<AuthEnv>): Promise<ConversationRecord | null> {
    const id = c.req.param('id')
    const conversation = id === undefined ? null : await control.getConversation(id)
    return conversation && conversation.userId === c.get('user').id ? conversation : null
  }

  async function call(conversation: ConversationRecord, reach: Reach, operation: string, args: Record<string, unknown>, signal: AbortSignal): Promise<unknown> {
    const bytes = await reach.host.services.call({
      context: await buildCommandContext(control, conversation.id, { kind: 'user' }),
      package: browser.package,
      operation: `browser.${operation}`,
      args,
      json: true,
      cwd: reach.cwd,
      input: new Uint8Array(),
      resolveArtifact: browser.resolveArtifact,
      maxBytes: ANSWER_MAX_BYTES,
      signal,
    })
    return JSON.parse(new TextDecoder().decode(bytes))
  }

  /** Without waking a stopped Cloud: null when the Host is not there to ask. */
  async function quietly<T>(conversation: ConversationRecord, signal: AbortSignal, run: (reach: Reach) => Promise<T>): Promise<T | null> {
    let access
    try {
      access = await targets.stream(conversation.id, signal)
    } catch (error) {
      if (error instanceof HostAccessRefused && error.code === 'host_stopped')
        return null
      throw error
    }
    try {
      return await run({ host: access.host, cwd: access.cwd })
    } finally {
      access.release()
    }
  }

  /** What a refusal answers: the Host access's own code, or the operation's. */
  function refused(c: Context<AuthEnv>, error: unknown): Response {
    if (error instanceof HostAccessRefused)
      return c.json({ code: error.code, message: error.message }, 409)
    if (errorCode(error) === 'ERUNNEROFFLINE')
      return c.json({ code: 'device_offline', message: 'The device has no live runner' }, 409)
    if (error instanceof RemoteServiceExit) {
      const failure = failureSchema.safeParse(parseJson(error.stderr))
      if (failure.success && failure.data.error.code === 'tab_not_found')
        return c.json({ code: 'tab_not_found', message: failure.data.error.message }, 404)
      const message = failure.success ? `${failure.data.error.code}: ${failure.data.error.message}` : error.message
      return c.json({ code: 'browser_failed', message }, 502)
    }
    if (error instanceof RemoteServiceError)
      return c.json({ code: 'browser_failed', message: error.message }, 502)
    throw error
  }

  const notFound = (c: Context<AuthEnv>) => c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
  const archived = (c: Context<AuthEnv>) => c.json({ code: 'conversation_archived', message: 'The conversation is archived' }, 409)

  app.get('/:id/browser/tabs', async (c) => {
    const conversation = await own(c)
    if (!conversation)
      return notFound(c)
    if (conversation.archived)
      return archived(c)
    try {
      const answer = await quietly(conversation, c.req.raw.signal, (reach) => call(conversation, reach, 'tabs', {}, c.req.raw.signal))
      // A stopped Cloud holds no browser.
      return c.json(answer === null ? { tabs: [] } : tabsAnswerSchema.parse(answer))
    } catch (error) {
      return refused(c, error)
    }
  })

  app.post('/:id/browser/tabs', async (c) => {
    const conversation = await own(c)
    if (!conversation)
      return notFound(c)
    if (conversation.archived)
      return archived(c)
    const body = z.strictObject({ url: z.string().max(4096).optional() }).safeParse(await c.req.json().catch(() => null))
    if (!body.success)
      return c.json({ code: 'invalid_body', message: 'Expected { url? }' }, 400)
    const url = body.data.url ?? 'about:blank'
    try {
      // Opening a tab is ordinary demand: it wakes a stopped Cloud.
      const cwd = (await resolveExecutionTarget(control, registry, conversation)).path
      const answer = await targets.withHost(
        conversation.id,
        (host) => call(conversation, { host, cwd }, 'open', { url }, c.req.raw.signal),
        { signal: c.req.raw.signal },
      )
      const opened = tabAnswerSchema.parse(answer)
      return c.json({ id: opened.tab, title: '', url: opened.url ?? url, createdBy: { kind: 'user' } })
    } catch (error) {
      return refused(c, error)
    }
  })

  app.delete('/:id/browser/tabs/:tab', async (c) => {
    const conversation = await own(c)
    if (!conversation)
      return notFound(c)
    const tab = c.req.param('tab')
    try {
      await quietly(conversation, c.req.raw.signal, (reach) => call(conversation, reach, 'close', { tab }, c.req.raw.signal))
    } catch (error) {
      // A tab the browser no longer has is closed.
      const answer = refused(c, error)
      if (answer.status !== 404)
        return answer
    }
    return c.body(null, 204)
  })

  /** An operation on one tab of a browser that must be there: a stopped Cloud refuses. */
  async function onTab(c: Context<AuthEnv>, operation: string, args: Record<string, unknown>): Promise<Response> {
    const conversation = await own(c)
    if (!conversation)
      return notFound(c)
    if (conversation.archived)
      return archived(c)
    const tab = c.req.param('tab')
    try {
      const answer = await quietly(conversation, c.req.raw.signal, (reach) => call(conversation, reach, operation, { tab, ...args }, c.req.raw.signal))
      if (answer === null)
        return c.json({ code: 'host_stopped', message: 'The Cloud is stopped' }, 409)
      return c.body(null, 204)
    } catch (error) {
      return refused(c, error)
    }
  }

  app.post('/:id/browser/tabs/:tab/navigate', async (c) => {
    const body = z.strictObject({ url: z.string().min(1).max(4096) }).safeParse(await c.req.json().catch(() => null))
    if (!body.success)
      return c.json({ code: 'invalid_body', message: 'Expected { url }' }, 400)
    return onTab(c, 'goto', { url: body.data.url })
  })

  app.post('/:id/browser/tabs/:tab/history', async (c) => {
    const body = z.strictObject({ action: z.enum(['back', 'forward', 'reload']) }).safeParse(await c.req.json().catch(() => null))
    if (!body.success)
      return c.json({ code: 'invalid_body', message: 'Expected { action: back | forward | reload }' }, 400)
    return onTab(c, body.data.action, {})
  })

  return app
}

function parseJson(text: string): unknown {
  try {
    return JSON.parse(text)
  } catch {
    // Standard error that is not the JSON failure shape is shown as it is.
    return null
  }
}
