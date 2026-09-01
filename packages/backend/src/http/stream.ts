import {
  createWebSocketServerTransport,
  type AgentServer,
  type AgentTransportBinding,
  type JsonWebSocket,
} from '@demicodes/agent'
import { Hono } from 'hono'
import type { UpgradeWebSocket } from 'hono/ws'
import type { WSContext } from 'hono/ws'
import { conversationScopedTransport } from '../conversation/scoped-transport'
import type { ControlService } from '../storage/control'

/**
 * `WS /api/conversations/:id/stream` — the live frame-protocol socket.
 * The conversation lookup happens in the route (Hono's upgrade handler
 * factory cannot reject); the frame scoping itself is conversation-module
 * logic in `scoped-transport.ts`.
 */
export function streamRoutes(options: {
  control: ControlService
  agentServer: AgentServer
  upgradeWebSocket: UpgradeWebSocket
}): Hono {
  const { control, agentServer, upgradeWebSocket } = options
  const app = new Hono()

  app.get('/:id/stream', async (c, next) => {
    const conversation = await control.getConversation(c.req.param('id'))
    if (!conversation) return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
    return upgradeWebSocket(() => {
      const adapter = new WsContextAdapter()
      let binding: AgentTransportBinding | null = null
      return {
        onOpen(_event, ws) {
          const transport = conversationScopedTransport(
            createWebSocketServerTransport(adapter.socket(ws)),
            conversation,
            control,
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
