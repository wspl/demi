import { webAssetRoutes } from './web-assets'
import {
  runnerInstallRoutes,
  type RunnerInstallationOptions
} from './runner-install'
import type { AgentServer } from '@demicodes/agent'
import { Hono } from 'hono'
import type { UpgradeWebSocket } from 'hono/ws'
import type { ProviderAssembly } from '../llm/assembly'
import type { RunnerRegistry } from '../runner/registry'
import type { PipeBroker } from '../runner/pipes'
import type { ControlService, WorkspaceRecord } from '../storage/control'
import type { ConversationStores } from '../storage/conversation-store'
import type { ProviderVault } from '../vault/providers'
import type { SubscriptionLoginFlows } from '../vault/subscription-login'
import type { UserBlobStores } from '../storage/user-blobs'
import type { Host } from '@demicodes/shell'
import type { ManagedHosts } from '../managed/lifecycle'
import type { LoginLimiter } from '../auth/login-limiter'
import type { WebSessions } from '../auth/sessions'
import { authenticate } from './authenticate'
import { setupRoutes } from './setup'
import type { ProductState } from '../sync/product-state'
import { stateRoutes } from './state'
import { sidebarRoutes } from './sidebar'
import { settingsRoutes } from './settings'
import { userRoutes } from './users'
import type { InstanceMode } from '../auth/identity'
import { attachmentRoutes } from './attachments'
import type { EmailChanges } from '../auth/email-change'
import { emailChangeRoutes } from './email-change'
import { authRoutes } from './auth'
import { blobRoutes } from './blobs'
import type { ProviderAccounts } from '../vault/provider-accounts'
import type { ProviderOperations } from '../vault/provider-operations'
import { providerRoutes } from './providers'
import type { VendorCatalog } from '../llm/vendors'
import type { ConversationUpdates } from '../conversation/updates'
import type { ConversationForks } from '../conversation/fork'
import { conversationRoutes } from './conversations'
import { deviceRoutes } from './devices'
import { modelRoutes } from './models'
import { runnerSocketRoutes } from './runner-socket'
import { pipeRoutes } from './pipes'
import { streamRoutes } from './stream'
import { usageRoutes } from './usage'
import { cloudRoutes } from './cloud'
import { workspaceRoutes } from './workspaces'

/**
 * Assembles the external HTTP surface: error shape, 404 shape, one route module
 * per resource.
 */
export function createApp(options: {
  webDirectory?: string
  runnerInstallation?: RunnerInstallationOptions
  productState: ProductState
  conversationUpdates: ConversationUpdates
  conversationForks: ConversationForks
  admitFrame: (id: string) => (() => void) | null
  control: ControlService
  conversationStores: ConversationStores
  vault: ProviderVault
  assembly: ProviderAssembly
  vendors: VendorCatalog
  logins: SubscriptionLoginFlows
  providerOperations: ProviderOperations
  providerAccounts: ProviderAccounts
  agentServer: AgentServer
  runnerRegistry: RunnerRegistry
  pipes: PipeBroker
  upgradeWebSocket: UpgradeWebSocket
  blobs: UserBlobStores
  withHost: <T>(
    conversationId: string,
    operation: (host: Host) => Promise<T>,
    signal?: AbortSignal
  ) => Promise<T>
  managedHosts: ManagedHosts | null
  createCloudWorkspace: ((
    userId: string,
    name: string
  ) => Promise<WorkspaceRecord>) | null
  sessions: WebSessions
  loginLimiter: LoginLimiter
  emailChanges: EmailChanges
  mode: InstanceMode
}): Hono {
  const app = new Hono()

  app.onError(
    (error, c) => c.json(
      { code: 'internal_error', message: error.message },
      500
    )
  )
  app.notFound((c) => c.json({
    code: 'not_found',
    message: `No route for ${c.req.method} ${c.req.path}`
  }, 404))

  // Everything under /api needs a session except the two entrances and the
  // routes runners dial with their device token.
  app.use('/api/*', authenticate(options.sessions, [
    '/api/setup',
    '/api/auth/login',
    '/api/runner',
    '/api/pipes'
  ]))

  app.route('/', runnerInstallRoutes(options.runnerInstallation))
  app.route('/api/setup', setupRoutes({
    control: options.control,
    sessions: options.sessions
  }))
  app.route(
    '/api/auth/email',
    emailChangeRoutes(options.emailChanges, options.control)
  )
  app.route('/api/auth', authRoutes({
    control: options.control,
    sessions: options.sessions,
    limiter: options.loginLimiter
  }))
  app.route('/api/users', userRoutes({ control: options.control }))
  app.route('/api/state', stateRoutes(options.productState))
  app.route('/api/sidebar', sidebarRoutes(options.control))
  app.route(
    '/api/settings',
    settingsRoutes({ mode: options.mode, control: options.control })
  )
  app.route('/api/models', modelRoutes({
    assembly: options.assembly,
    mode: options.mode
  }))
  app.route('/api/providers', providerRoutes({
    accounts: options.providerAccounts,
    operations: options.providerOperations,
    vault: options.vault,
    assembly: options.assembly,
    vendors: options.vendors,
    logins: options.logins,
    mode: options.mode
  }))
  app.route(
    '/api/usage',
    usageRoutes({ control: options.control, mode: options.mode })
  )
  app.route('/api/runner', runnerSocketRoutes({
    registry: options.runnerRegistry,
    upgradeWebSocket: options.upgradeWebSocket
  }))
  app.route('/api/pipes', pipeRoutes({
    control: options.control,
    broker: options.pipes
  }))
  app.route('/api/devices', deviceRoutes({
    control: options.control,
    registry: options.runnerRegistry
  }))
  app.route('/api/cloud', cloudRoutes(options.managedHosts))
  app.route('/api/workspaces', workspaceRoutes({
    control: options.control,
    managedHosts: options.managedHosts,
    createCloudWorkspace: options.createCloudWorkspace
  }))
  app.route('/api/attachments', attachmentRoutes({
    control: options.control,
    blobsFor: (id) => options.blobs.forUser(id)
  }))
  app.route(
    '/api/blobs',
    blobRoutes({ blobsFor: (id) => options.blobs.forUser(id) })
  )
  // The stream route registers first so `/:id/stream` wins over the REST group's `/:id/*`.
  app.route(
    '/api/conversations',
    streamRoutes({
      admitFrame: options.admitFrame,
      assembly: options.assembly,
      registry: options.runnerRegistry,
      control: options.control,
      agentServer: options.agentServer,
      upgradeWebSocket: options.upgradeWebSocket,
      blobsFor: (id) => options.blobs.forUser(id),
      withHost: options.withHost,
      vault: options.vault,
      mode: options.mode,
    }),
  )
  app.route(
    '/api/conversations',
    conversationRoutes({
      admitFrame: options.admitFrame,
      updates: options.conversationUpdates,
      forks: options.conversationForks,
      agentServer: options.agentServer,
      control: options.control,
      conversationStores: options.conversationStores,
      withHost: options.withHost,
      registry: options.runnerRegistry,
    }),
  )

  app.route('/', webAssetRoutes(options.webDirectory))
  return app
}
