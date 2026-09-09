import {
  createWebSocketServerTransport,
  type AgentServer,
  type AgentTransportBinding,
  type BlobStore,
  type JsonWebSocket,
} from '@demicodes/agent'
import type { ProviderAssembly } from '../llm/assembly'
import { Hono } from 'hono'
import type { UpgradeWebSocket } from 'hono/ws'
import type { WSContext } from 'hono/ws'
import type { RunnerRegistry } from '../runner/registry'
import { resolveExecutionTarget } from '../conversation/execution-target'
import { conversationScopedTransport } from '../conversation/scoped-transport'
import type { ControlService } from '../storage/control'
import type { AuthEnv, InstanceMode } from '../auth/identity'
import type { ProviderVault } from '../vault/providers'
import { visibleProvider } from '../vault/scope'

/**
 * `WS /api/conversations/:id/stream` — the live frame-protocol socket.
 * The conversation lookup happens in the route (Hono's upgrade handler
 * factory cannot reject); the frame scoping itself is conversation-module
 * logic in `scoped-transport.ts`.
 */
export function streamRoutes(options: {
  assembly: ProviderAssembly
  registry: RunnerRegistry
  control: ControlService
  agentServer: AgentServer
  upgradeWebSocket: UpgradeWebSocket
  blobsFor: (userId: string) => BlobStore
  vault: ProviderVault
  mode: InstanceMode
}): Hono<AuthEnv> {
  const { control, agentServer, upgradeWebSocket, blobsFor, vault, mode } = options
  const app = new Hono<AuthEnv>()

  app.get('/:id/stream', async (c, next) => {
    const conversation = await control.getConversation(c.req.param('id'))
    if (!conversation || conversation.userId !== c.get('user').id) return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
    const target = await resolveExecutionTarget(control, options.registry, conversation)
    return upgradeWebSocket(() => {
      const adapter = new WsContextAdapter()
      let binding: AgentTransportBinding | null = null
      return {
        onOpen(_event, ws) {
          const transport = conversationScopedTransport(createWebSocketServerTransport(adapter.socket(ws)), conversation, {
            control,
            modelSelection: (providerId, selection) => options.assembly.selection(providerId, selection),
            cwd: target.path,
            blobs: blobsFor(conversation.userId),
            providerAllowed: async (providerId) => (await visibleProvider(vault, mode, conversation.userId, providerId)) !== null,
          })
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

  return app
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
