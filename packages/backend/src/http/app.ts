import type { AgentServer } from '@demicodes/agent'
import { Hono } from 'hono'
import type { UpgradeWebSocket } from 'hono/ws'
import type { ProviderAssembly } from '../llm/assembly'
import type { RunnerRegistry } from '../runner/registry'
import type { TransferBroker } from '../runner/transfers'
import type { ControlService, WorkspaceRecord } from '../storage/control'
import type { ConversationStores } from '../storage/conversation-store'
import type { ProviderVault } from '../vault/providers'
import type { SubscriptionLoginFlows } from '../vault/subscription-login'
import type { BlobStore } from '@demicodes/agent'
import type { Host } from '@demicodes/shell'
import type { ManagedHosts } from '../managed/lifecycle'
import type { LoginLimiter } from '../auth/login-limiter'
import type { WebSessions } from '../auth/sessions'
import { authenticate } from './authenticate'
import { setupRoutes } from './setup'
import { settingsRoutes } from './settings'
import { userRoutes } from './users'
import type { InstanceMode } from '../auth/identity'
import { attachmentRoutes } from './attachments'
import { authRoutes } from './auth'
import { blobRoutes } from './blobs'
import { providerRoutes } from './providers'
import { conversationRoutes } from './conversations'
import { deviceRoutes } from './devices'
import { modelRoutes } from './models'
import { runnerSocketRoutes } from './runner-socket'
import { transferRoutes } from './transfers'
import { streamRoutes } from './stream'
import { usageRoutes } from './usage'
import { workspaceRoutes } from './workspaces'

/** Assembles the external HTTP surface: error shape, 404 shape, one route module per resource. */
export function createApp(options: {
  control: ControlService
  conversationStores: ConversationStores
  vault: ProviderVault
  assembly: ProviderAssembly
  logins: SubscriptionLoginFlows
  agentServer: AgentServer
  runnerRegistry: RunnerRegistry
  transfers: TransferBroker
  upgradeWebSocket: UpgradeWebSocket
  blobs: BlobStore
  hostFor: (conversationId: string) => Promise<Host>
  managedHosts: ManagedHosts | null
  createCloudWorkspace: ((userId: string, name: string) => Promise<WorkspaceRecord>) | null
  sessions: WebSessions
  loginLimiter: LoginLimiter
  mode: InstanceMode
}): Hono {
  const app = new Hono()

  app.onError((error, c) => c.json({ code: 'internal_error', message: error.message }, 500))
  app.notFound((c) => c.json({ code: 'not_found', message: `No route for ${c.req.method} ${c.req.path}` }, 404))

  // Everything under /api needs a session except the two entrances and the
  // routes runners dial with their device token.
  app.use('/api/*', authenticate(options.sessions, ['/api/setup', '/api/auth/login', '/api/runner', '/api/transfers']))

  app.route('/api/setup', setupRoutes({ control: options.control, sessions: options.sessions }))
  app.route('/api/auth', authRoutes({ control: options.control, sessions: options.sessions, limiter: options.loginLimiter }))
  app.route('/api/users', userRoutes({ control: options.control }))
  app.route('/api/settings', settingsRoutes({ mode: options.mode }))
  app.route('/api/models', modelRoutes({ assembly: options.assembly, mode: options.mode }))
  app.route('/api/providers', providerRoutes({ vault: options.vault, assembly: options.assembly, logins: options.logins, mode: options.mode }))
  app.route('/api/usage', usageRoutes({ control: options.control, mode: options.mode }))
  app.route('/api/runner', runnerSocketRoutes({ registry: options.runnerRegistry, upgradeWebSocket: options.upgradeWebSocket }))
  app.route('/api/transfers', transferRoutes({ control: options.control, broker: options.transfers }))
  app.route('/api/devices', deviceRoutes({ control: options.control, registry: options.runnerRegistry }))
  app.route('/api/workspaces', workspaceRoutes({ control: options.control, managedHosts: options.managedHosts, createCloudWorkspace: options.createCloudWorkspace }))
  app.route('/api/attachments', attachmentRoutes({ control: options.control, blobs: options.blobs }))
  app.route('/api/blobs', blobRoutes({ blobs: options.blobs }))
  // The stream route registers first so `/:id/stream` wins over the REST group's `/:id/*`.
  app.route(
    '/api/conversations',
    streamRoutes({
      control: options.control,
      agentServer: options.agentServer,
      upgradeWebSocket: options.upgradeWebSocket,
      blobs: options.blobs,
    }),
  )
  app.route(
    '/api/conversations',
    conversationRoutes({
      control: options.control,
      conversationStores: options.conversationStores,
      agentServer: options.agentServer,
      hostFor: options.hostFor,
      managedHosts: options.managedHosts,
      vault: options.vault,
      mode: options.mode,
    }),
  )

  return app
}
