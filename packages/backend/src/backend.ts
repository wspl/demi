import { join } from 'node:path'
import {
  AgentServer,
  createWebSocketServerTransport,
  type AgentServerTransport,
  type AgentTransportBinding,
  type ClientFrame,
  type JsonWebSocket,
} from '@demicodes/agent'
import { createCodingAgentHarness } from '@demicodes/coding-agent'
import { LocalHost } from '@demicodes/host-local'
import { VirtualHost, scopedFsBackend } from '@demicodes/host-virtual'
import type { Provider, ProviderModel } from '@demicodes/provider'
import type { Host } from '@demicodes/shell'
import { Hono } from 'hono'
import { createBunWebSocket } from 'hono/bun'
import type { WSContext } from 'hono/ws'
import { ConversationIndex, type ConversationRecord } from './storage/conversations'
import { openSqliteDatabase, type SqlDatabase } from './storage/database'
import { DbHostStore } from './storage/host-store'
import { migrate } from './storage/migrations'

/** Virtual working directory every virtual-target conversation starts in. */
export const VIRTUAL_WORKSPACE_CWD = '/workspace'

const STUB_USER = { id: 'local', username: 'local', role: 'master' } as const

export interface BackendOptions {
  /** Data directory: SQLite file + per-conversation virtual filesystems. */
  dataDir: string
  /** Operator-assembled providers (the vault arrives in M3). */
  providers: Provider[]
  /** HTTP port (0 = ephemeral, for tests). */
  port?: number
}

export interface Backend {
  port: number
  url: string
  close(): Promise<void>
}

export async function createBackend(options: BackendOptions): Promise<Backend> {
  const db = openSqliteDatabase(join(options.dataDir, 'demi.sqlite'))
  migrate(db)
  ensureStubUser(db)

  const conversations = new ConversationIndex(db)
  const store = new DbHostStore(db, 'agent')
  const localFs = new LocalHost(options.dataDir).fs

  // One stable VirtualHost per conversation — AgentHarness.host must return
  // the same object for the same execution target (per-Host shell reuse).
  const virtualHosts = new Map<string, Promise<VirtualHost>>()
  const virtualHostFor = (conversationId: string): Promise<VirtualHost> => {
    let host = virtualHosts.get(conversationId)
    if (!host) {
      host = (async () => {
        const virtual = new VirtualHost({
          backend: scopedFsBackend(join(options.dataDir, 'virtual', conversationId), localFs),
          store,
        })
        await virtual.ensureLayout()
        return virtual
      })()
      virtualHosts.set(conversationId, host)
    }
    return host
  }

  const agentServer = new AgentServer({
    agent: createCodingAgentHarness({
      host: (ctx): Promise<Host> =>
        // Shell/reference contexts carry the session id (= conversation id);
        // session-less contexts only touch the shared store, so any namespace works.
        virtualHostFor('agentSessionId' in ctx ? ctx.agentSessionId : 'lobby'),
    }),
    providers: options.providers,
    shell: { initialEnv: { PATH: '/usr/bin:/bin' } },
  })

  const { upgradeWebSocket, websocket } = createBunWebSocket()
  const app = new Hono()

  app.onError((error, c) => c.json({ code: 'internal_error', message: error.message }, 500))
  app.notFound((c) => c.json({ code: 'not_found', message: `No route for ${c.req.method} ${c.req.path}` }, 404))

  app.get('/api/auth/me', (c) => c.json({ user: STUB_USER }))
  app.post('/api/auth/login', (c) => c.json({ user: STUB_USER }))
  app.post('/api/auth/logout', (c) => c.body(null, 204))

  app.get('/api/conversations', (c) => {
    const archived = c.req.query('archived') === 'true'
    return c.json({ conversations: conversations.listForUser(STUB_USER.id, { archived }) })
  })
  app.post('/api/conversations', (c) => c.json({ conversation: conversations.create(STUB_USER.id) }, 201))

  app.patch('/api/conversations/:id', async (c) => {
    const conversation = conversations.get(c.req.param('id'))
    if (!conversation) return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
    const body = await c.req.json<{
      title?: string
      archived?: boolean
      connectionId?: string | null
      modelId?: string | null
    }>()
    if (typeof body.title === 'string' && body.title.trim()) conversations.rename(conversation.id, body.title.trim())
    if (typeof body.archived === 'boolean') conversations.setArchived(conversation.id, body.archived)
    if (body.connectionId !== undefined || body.modelId !== undefined) {
      conversations.setModel(conversation.id, body.connectionId ?? null, body.modelId ?? null)
    }
    return c.json({ conversation: conversations.get(conversation.id) })
  })

  app.get('/api/conversations/:id/transcript', async (c) => {
    const conversation = conversations.get(c.req.param('id'))
    if (!conversation) return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
    const checkpoint = await store.readJson<{ transcript?: { blocks?: unknown[] } }>(
      `agent-sessions/${conversation.id}/checkpoint.json`,
    )
    return c.json({ blocks: checkpoint?.transcript?.blocks ?? [] })
  })

  app.get('/api/models', async (c) => {
    const connections = await Promise.all(
      options.providers.map(async (provider) => ({
        connectionId: provider.id,
        displayName: provider.displayName,
        requiresProcessCapableHost: provider.requiresProcessCapableHost ?? false,
        models: ((await provider.listModels?.()) ?? { models: [] as ProviderModel[] }).models,
      })),
    )
    return c.json({ connections })
  })

  app.get('/api/conversations/:id/stream', async (c, next) => {
    const conversation = conversations.get(c.req.param('id'))
    if (!conversation) return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
    return upgradeWebSocket(() => {
      const adapter = new WsContextAdapter()
      let binding: AgentTransportBinding | null = null
      return {
        onOpen(_event, ws) {
          const transport = conversationScopedTransport(
            createWebSocketServerTransport(adapter.socket(ws)),
            conversation,
            conversations,
          )
          binding = agentServer.attachTransport(transport)
        },
        onMessage(event) {
          adapter.deliver(typeof event.data === 'string' ? event.data : String(event.data))
        },
        onClose() {
          void binding?.close()
        },
      }
    })(c, next)
  })

  const server = Bun.serve({
    port: options.port ?? 0,
    fetch: app.fetch,
    websocket,
  })

  return {
    port: server.port ?? 0,
    url: `http://localhost:${server.port}`,
    close: async () => {
      await agentServer.close()
      server.stop(true)
      db.close()
    },
  }
}

function ensureStubUser(db: SqlDatabase): void {
  db.run(
    'INSERT INTO users (id, username, password_hash, role, created_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT (id) DO NOTHING',
    [STUB_USER.id, STUB_USER.username, '!', STUB_USER.role, new Date().toISOString()],
  )
}

/**
 * Scopes an incoming stream to its conversation: the session id and cwd are
 * resolved server-side from the conversation record (the browser never names
 * a cwd), the first user message becomes the default title, and activity
 * bumps the index row.
 */
function conversationScopedTransport(
  inner: AgentServerTransport,
  conversation: ConversationRecord,
  conversations: ConversationIndex,
): AgentServerTransport {
  return {
    send: (frame) => inner.send(frame),
    onFrame: (handler) =>
      inner.onFrame((frame) => {
        handler(rewriteFrame(frame, conversation, conversations))
      }),
    close: () => inner.close(),
  }
}

function rewriteFrame(
  frame: ClientFrame,
  conversation: ConversationRecord,
  conversations: ConversationIndex,
): ClientFrame {
  if (frame.type === 'open') {
    conversations.setModel(conversation.id, frame.provider.providerId, frame.provider.model.model.id)
    return { ...frame, sessionId: conversation.id, cwd: VIRTUAL_WORKSPACE_CWD }
  }
  if (frame.type === 'send') {
    const text = frame.content.find((block): block is { type: 'text'; text: string } => block.type === 'text')?.text
    const title = (text ?? '').replace(/\s+/g, ' ').trim().slice(0, 80)
    if (title) conversations.defaultTitle(conversation.id, title)
    conversations.touch(conversation.id)
    return frame
  }
  if (frame.type === 'set_provider') {
    conversations.setModel(conversation.id, frame.provider.providerId, frame.provider.model.model.id)
    return frame
  }
  return frame
}

/** Adapts Hono's WSContext to the agent transport's socket shape. */
class WsContextAdapter {
  private readonly listeners = new Set<(event: { data: unknown }) => void>()

  socket(ws: WSContext): JsonWebSocket {
    return {
      send: (data) => {
        ws.send(data)
      },
      close: () => {
        ws.close()
      },
      addEventListener: (_type, listener) => {
        this.listeners.add(listener)
      },
      removeEventListener: (_type, listener) => {
        this.listeners.delete(listener)
      },
    }
  }

  deliver(data: string): void {
    for (const listener of [...this.listeners]) listener({ data })
  }
}
