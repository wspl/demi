import {
  createWebSocketTransport,
  type ServerFrame,
  type DisplayedBlock,
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
import { resolveRemoteFileRefs } from '../conversation/remote-file-refs'
import { conversationClientFrameSchema, type ConversationClientFrame } from '../conversation/client-frames'
import { conversationScopedTransport } from '../conversation/scoped-transport'
import type { ControlService } from '../storage/control'
import type { AuthEnv, InstanceMode } from '../auth/identity'
import type { ProviderVault } from '../vault/providers'
import { visibleProvider } from '../vault/scope'
import { writeAttachmentToHost } from '../conversation/attachment-refs'
import type { Host } from '@demicodes/shell'

/**
 * `WS /api/conversations/:id/stream` — the live frame-protocol socket.
 * The conversation lookup happens in the route (Hono's upgrade handler
 * factory cannot reject); the frame scoping itself is conversation-module
 * logic in `scoped-transport.ts`.
 */
export function streamRoutes(options: {
  admitFrame: (id: string) => (() => void) | null
  assembly: ProviderAssembly
  registry: RunnerRegistry
  control: ControlService
  agentServer: AgentServer
  upgradeWebSocket: UpgradeWebSocket
  blobsFor: (userId: string) => BlobStore
  withHost: <T>(
    conversationId: string,
    operation: (host: Host) => Promise<T>,
    signal?: AbortSignal,
  ) => Promise<T>
  vault: ProviderVault
  mode: InstanceMode
}): Hono<AuthEnv> {
  const { control, agentServer, upgradeWebSocket, blobsFor, vault, mode } = options
  const app = new Hono<AuthEnv>()

  app.get('/:id/stream', async (c, next) => {
    const conversation = await control.getConversation(c.req.param('id'))
    if (!conversation || conversation.userId !== c.get('user').id)
      return c.json({
        code: 'conversation_not_found',
        message: 'No such conversation'
      }, 404)
    if (conversation.archived)
      return c.json({
        code: 'archived',
        message: 'Use the transcript endpoint for archived history'
      }, 409)
    const target = await resolveExecutionTarget(
      control,
      options.registry,
      conversation
    )
    return upgradeWebSocket(() => {
      const adapter = new WsContextAdapter()
      let binding: AgentTransportBinding | null = null
      return {
        onOpen(_event, ws) {
          const transport = conversationScopedTransport(
            createWebSocketTransport<ServerFrame<DisplayedBlock>, ConversationClientFrame>(
              adapter.socket(ws),
              { decode: conversationClientFrameSchema.parse },
            ),
            conversation,
            {
              control,
              resolveRemoteFiles: (record, content) => resolveRemoteFileRefs(
                control,
                options.registry,
                record,
                content
              ),
              admitFrame: () => options.admitFrame(conversation.id),
              modelSelection: (providerId, selection) => options.assembly.selection(
                providerId,
                selection
              ),
              cwd: target.path,
              blobs: blobsFor(conversation.userId),
              writeAttachment: (fileName, data) => options.withHost(
                conversation.id,
                (host) => writeAttachmentToHost(host, conversation.id, fileName, data)
              ),
              providerAllowed: async (providerId) => (await visibleProvider(
                vault,
                mode,
                conversation.userId,
                providerId
              )) !== null,
            }
          )
          binding = agentServer.attachTransport(transport)
        },
        onMessage(event) {
          adapter.deliver('message', event.data)
        },
        onClose() {
          adapter.deliver('close')
          void binding?.close()
        },
        onError() {
          adapter.deliver('error')
        },
      }
    })(c, next)
  })

  return app
}

/** Adapts Hono's WSContext to the agent transport's socket shape. */
class WsContextAdapter {
  private readonly listeners = new Map<string, Set<(event: { data: unknown }) => void>>()

  socket(ws: WSContext): JsonWebSocket {
    return {
      send: (data) => {
        ws.send(data)
      },
      close: () => {
        ws.close()
      },
      addEventListener: (type, listener) => {
        const listeners = this.listeners.get(type) ?? new Set()
        listeners.add(listener)
        this.listeners.set(type, listeners)
      },
      removeEventListener: (type, listener) => {
        this.listeners.get(type)?.delete(listener)
      },
    }
  }

  deliver(type: 'message' | 'close' | 'error', data?: unknown): void {
    for (const listener of this.listeners.get(type) ?? [])
      listener({ data })
  }
}
