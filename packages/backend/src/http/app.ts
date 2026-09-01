import type { AgentServer } from '@demicodes/agent'
import { Hono } from 'hono'
import type { UpgradeWebSocket } from 'hono/ws'
import type { ProviderAssembly } from '../llm/assembly'
import type { RunnerRegistry } from '../runner/registry'
import type { ControlService } from '../storage/control'
import type { ConversationStores } from '../storage/conversation-store'
import type { ConnectionVault } from '../vault/connections'
import type { SubscriptionLoginFlows } from '../vault/subscription-login'
import { authRoutes } from './auth'
import { connectionRoutes } from './connections'
import { conversationRoutes } from './conversations'
import { deviceRoutes } from './devices'
import { modelRoutes } from './models'
import { runnerSocketRoutes } from './runner-socket'
import { streamRoutes } from './stream'
import { usageRoutes } from './usage'

/** Assembles the external HTTP surface: error shape, 404 shape, one route module per resource. */
export function createApp(options: {
  control: ControlService
  conversationStores: ConversationStores
  vault: ConnectionVault
  assembly: ProviderAssembly
  logins: SubscriptionLoginFlows
  agentServer: AgentServer
  runnerRegistry: RunnerRegistry
  upgradeWebSocket: UpgradeWebSocket
}): Hono {
  const app = new Hono()

  app.onError((error, c) => c.json({ code: 'internal_error', message: error.message }, 500))
  app.notFound((c) => c.json({ code: 'not_found', message: `No route for ${c.req.method} ${c.req.path}` }, 404))

  app.route('/api/auth', authRoutes())
  app.route('/api/models', modelRoutes(options.assembly))
  app.route('/api/connections', connectionRoutes({ vault: options.vault, assembly: options.assembly, logins: options.logins }))
  app.route('/api/usage', usageRoutes({ control: options.control }))
  app.route('/api/runner', runnerSocketRoutes({ registry: options.runnerRegistry, upgradeWebSocket: options.upgradeWebSocket }))
  app.route('/api/devices', deviceRoutes({ control: options.control, registry: options.runnerRegistry }))
  // The stream route registers first so `/:id/stream` wins over the REST group's `/:id/*`.
  app.route(
    '/api/conversations',
    streamRoutes({
      control: options.control,
      agentServer: options.agentServer,
      upgradeWebSocket: options.upgradeWebSocket,
    }),
  )
  app.route(
    '/api/conversations',
    conversationRoutes({ control: options.control, conversationStores: options.conversationStores }),
  )

  return app
}
