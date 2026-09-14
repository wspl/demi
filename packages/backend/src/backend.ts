import { ModelCatalogCache } from './llm/model-catalog-cache'
import { join } from 'node:path'
import {
  AgentServer,
  type ShellEnvironmentFactory
} from '@demicodes/agent'
import {
  createCodingAgentHarness,
  createDemiCommand
} from '@demicodes/coding-agent'
import {
  buildManifest,
  inProcessRpc,
} from '@demicodes/command-loader'
import { RemoteHost, createRemoteShellEnvironmentFactory, type RemoteShellEnvironmentFactoryOptions } from '@demicodes/host-remote'
import { ConversationBrowsers } from './conversation/browsers'
import { LifecycleCoordinator } from './lifecycle/coordinator'
import { CloudConversations } from './managed/conversations'
import {
  type Command,
  type CommandIO,
  type CommandRegistry
} from '@demicodes/shell'
import { toBytes } from '@demicodes/utils'
import type { Host } from '@demicodes/shell'
import { createBunWebSocket } from 'hono/bun'
import type { InstanceMode } from './auth/identity'
import { EmailChanges, type AccountMailSender } from './auth/email-change'
import { LoginLimiter, type LoginLimiterOptions } from './auth/login-limiter'
import { ownerFitsMode } from './vault/scope'
import { WebSessions, type WebSessionsOptions } from './auth/sessions'
import { switchAnnouncementPreamble } from './conversation/switch-announcement'
import { ProductState } from './sync/product-state'
import { ConversationUpdates } from './conversation/updates'
import { ConversationForks } from './conversation/fork'
import { ConversationTargets } from './conversation/target'
import { CLOUD_HOME } from './conversation/execution-target'
import { createCloudWorkspace } from './managed/cloud-workspace'
import { createHostCommandGroup } from './runner/host-command'
import { ManagedHosts, type ManagedHostsConfig } from './managed/lifecycle'
import type { ManagedHostProvisioner } from '@demicodes/machines'
import { createApp } from './http/app'
import {
  ProviderAssembly,
  builtinProviderTypes,
  type ProviderType
} from './llm/assembly'
import { VendorCatalog } from './llm/vendors'
import type { ModelsDevFetch } from '@demicodes/provider'
import { createSessionProviderResolver } from './llm/session-providers'
import { RunnerRegistry, type RunnerRegistryOptions } from './runner/registry'
import { PipeBroker } from './runner/pipes'
import { ProviderRateLimiter } from './usage/rate-limit'
import { ProviderVault } from './vault/providers'
import { loadOrCreateInstanceSecret } from './vault/secret'
import { ProviderAccounts } from './vault/provider-accounts'
import { ProviderOperations } from './vault/provider-operations'
import { SubscriptionLoginFlows } from './vault/subscription-login'
import { UserBlobStores } from './storage/user-blobs'
import { ChangeStore } from './storage/change-store'
import { DirChangeObjects, type ChangeObjects } from './storage/change-objects'
import { ConversationStores } from './storage/conversation-store'
import { LocalControlService, type ControlService } from './storage/control'
import { openSqliteDatabase } from './storage/database'
import { CONTROL_MIGRATIONS, migrate } from './storage/migrations'

export interface BackendOptions {
  /** Published historical edits; the caller owns an injected storage client. */
  changeObjects?: ChangeObjects
  /** Exact native package catalog and deployment-owned artifact resolution. */
  nativeCommands: RemoteShellEnvironmentFactoryOptions
  /**
   * Directory produced by scripts/native/release-runner.ts; exposes paired
   * client/runner downloads.
   */
  runnerReleaseDir?: string
  /** Optional directory containing a built browser application. */
  webDirectory?: string
  /** Delivers account email verification codes. */
  accountMail?: AccountMailSender
  /**
   * Data directory: control database, conversation databases, blobs and machine
   * images.
   */
  dataDir: string
  /** The instance mode, a deployment decision: `DEMI_INSTANCE_MODE`. */
  mode: InstanceMode
  /** HTTP port (0 = ephemeral, for tests). */
  port?: number
  /**
   * The URL managed guests dial (`managed-hosts.md` § Network); default the
   * local one, which only the fake's guests can reach.
   */
  publicUrl?: string
  /** Runner-management tuning (claim TTL, liveness interval) — tests only. */
  runner?: Omit<RunnerRegistryOptions, 'control'>
  /**
   * Extra provider families merged over the builtins — tests register stubs
   * here.
   */
  providerTypes?: Record<string, ProviderType>
  /** The models.dev fetch behind the vendor catalog — tests serve a fixture. */
  modelsDev?: {
    fetch?: ModelsDevFetch;
    url?: string
  }
  /** Browser idle-deadline tuning — tests only. */
  browser?: { idleMs?: number }
  /** Usage-enforcement tuning — tests only. */
  usage?: { providerRequestsPerMinute?: number }
  /** Session lifetime and login lockout tuning — tests only. */
  auth?: WebSessionsOptions & LoginLimiterOptions
  /**
   * Managed hosts (`managed-hosts.md`): the provisioner and the lifecycle
   * sizes. Cloud operations require it; paired devices remain independently usable.
   */
  managedHosts?: {
    provisioner: ManagedHostProvisioner;
    config?: Partial<ManagedHostsConfig>
  }
}

export interface Backend {
  port: number
  url: string
  /**
   * The lifecycle, when configured. Cloud operations allocate and wake machines
   * on demand.
   */
  managedHosts: ManagedHosts | null
  close(): Promise<void>
}

/**
 * The composition root: opens storage, assembles the services, mounts the HTTP
 * surface.
 */
export async function createBackend(options: BackendOptions): Promise<Backend> {
  const lifecycle = new LifecycleCoordinator()
  const createShellEnvironment = createRemoteShellEnvironmentFactory(options.nativeCommands)
  const browserPackage = options.nativeCommands.packages.find(descriptor => descriptor.id === 'demi.builtin' && descriptor.operations.includes('browser.open'))
  const controlDb = openSqliteDatabase(join(options.dataDir, 'control.sqlite'))
  migrate(controlDb, CONTROL_MIGRATIONS)
  const control: ControlService = new LocalControlService(controlDb)
  // The mode is fixed for the instance's life: providers configured under
  // the other mode would change owner meaning, so the process refuses to start.
  const misfits = (await control.listProviders('all')).filter(
    (row) => !ownerFitsMode(options.mode, row.ownerUserId)
  )
  if (misfits.length > 0) {
    controlDb.close()
    throw new Error(
      `${misfits.length} provider(s) were configured under the other instance mode; the mode cannot change once providers are configured`
    )
  }
  const sessions = new WebSessions(control, options.auth)
  const loginLimiter = new LoginLimiter(options.auth)

  const blobs = new UserBlobStores(join(options.dataDir, 'blobs'), control)
  const changes = new ChangeStore(options.changeObjects ?? new DirChangeObjects(options.dataDir))
  const conversationStores = new ConversationStores(
    join(options.dataDir, 'conversations'),
    (id) => blobs.forConversation(id)
  )

  // The command tree, defined once: the manifest every runner caches is built
  // from it plus the shape of the `agent` node every session grafts on. An
  // rpc command a runner relays runs as the session the job's environment
  // names — a conversation or a subagent — against the tree and the Host its
  // shell was built with.
  const sessionCommands = new Map<string, {
    rootSessionId: string;
    commands: CommandRegistry
  }>()
  const pipes = new PipeBroker()
  const runnerRegistry = new RunnerRegistry({
    admit: deviceId => managedHosts?.admit(deviceId) ?? (() => {}),
    control,
    pipes,
    // Bound late: the lifecycle is built over the registry below.
    volumeGrow: async (deviceId, volume, bytes) => {
      if (!managedHosts)
        throw new Error('this backend provisions no machines')
      await managedHosts.growVolume(deviceId, volume, bytes)
    },
    rpc: async (call, io, execution) => {
      const shell = sessionCommands.get(call.agentSessionId)
      if (!shell || shell.rootSessionId !== execution.conversationId)
        throw new Error(
          `no authorized session ${call.agentSessionId} behind this job`
        )
      if (!execution.commandStorage)
        throw new Error('rpc job has no agent command storage')
      const transport = inProcessRpc(shell.commands.list(), {
        storage: execution.commandStorage.withSignal(io.signal),
        host: execution.host,
      })
      const result = await transport({
        root: call.root,
        path: call.path,
        argv: call.argv,
        args: call.args,
        json: call.json,
        // The pipes are the caller's; a handler that attaches them to a job
        // elsewhere finds them on the io (`host shell`), the rest read and
        // write them here.
        stdin: io.stdin?.stream() ?? null,
        cwd: call.cwd,
        env: {
          ...call.env,
          DEMI_SESSION_ID: call.agentSessionId,
          DEMI_SHELL_ID: call.shellId
        },
        io: io.commandIO(),
        signal: io.signal,
        stdinStream: io.stdinStream,
      })
      return result.exitCode
    },
    ...options.runner,
  })

  const instanceSecret = loadOrCreateInstanceSecret(options.dataDir)
  const emailChanges = new EmailChanges(
    control,
    instanceSecret,
    options.accountMail,
    options.auth?.now
  )
  const vault = new ProviderVault(control, instanceSecret)
  const vaultRoot = join(options.dataDir, 'vault')
  const vendors = new VendorCatalog(options.modelsDev ?? {})
  const assembly = new ProviderAssembly(vault, {
    ...builtinProviderTypes(),
    ...options.providerTypes
  }, vaultRoot, vendors, new ModelCatalogCache(control))
  const providerOperations = new ProviderOperations()
  const logins = new SubscriptionLoginFlows(
    vault,
    assembly,
    { vaultRoot, operations: providerOperations }
  )
  const rateLimiter = new ProviderRateLimiter(
    options.usage?.providerRequestsPerMinute
  )

  const resolveProvider = createSessionProviderResolver({
    assembly,
    control,
    mode: options.mode,
    hostFor: (id) => targets.hostFor(id),
    rateLimiter
  })

  const cloudConversations = new CloudConversations({
    control,
    targets: () => targets,
    agents: () => agentServer,
    browsers: () => browsers,
  })
  const managedHosts = options.managedHosts
    ? new ManagedHosts({
      lifecycle,
      control,
      registry: runnerRegistry,
      provisioner: options.managedHosts.provisioner,
      config: options.managedHosts.config,
      backendUrl: () => options.publicUrl ?? url,
      turnInFlight: userId => cloudConversations.active(userId),
      observeActivity: (userId, changed) => cloudConversations.observe(userId, changed),
      reserveConversations: (userId, reason) => cloudConversations.reserve(userId, reason),
    })
    : null
  // Whatever a previous process left running or unsaved is settled before the first need can boot anything.
  await managedHosts?.reconcile()

  // One target resolver serves the root and all descendants.
  const targets = new ConversationTargets({
    lifecycle,
    retireBrowser: (id, reservation) => browsers?.retire(id, reservation) ?? Promise.resolve(),
    control,
    registry: runnerRegistry,
    managedHosts,
    stores: conversationStores,
    reserveTree: (conversationId) => agentServer.reserveTreeMutation(
      conversationId
    ),
  })

  const hostCommandDeps = {
    catalogFor: async (agentSessionId: string) => {
      const session = sessionCommands.get(agentSessionId)
      if (!session)
        throw new Error(`No command catalog for session ${agentSessionId}`)
      return {
        manifest: await buildManifest(session.commands.list(), { packages: options.nativeCommands.packages }),
        resolveArtifact: options.nativeCommands.resolveArtifact,
      }
    },
    control,
    registry: runnerRegistry,
    pipes,
    managedHosts,
    hostStoreFor: (conversationId: string) => conversationStores.hostStore(
      conversationId
    ),
  }
  const browsers = browserPackage ? new ConversationBrowsers({
    targets,
    idleMs: options.browser?.idleMs,
    descriptor: browserPackage,
    resolveArtifact: options.nativeCommands.resolveArtifact,
    lifecycle,
    treeActive: id => agentServer.treeActive(id),
    observeTree: (id, changed) => agentServer.observeTreeActivity(id, changed),
    reserveTree: id => agentServer.reserveTreeMutation(id),
  }) : null
  const commandsFor = (agentSessionId: string): Command[] => [
    createDemiCommand(
      { browser: browsers !== null, extraSubcommands: [createHostCommandGroup(
            hostCommandDeps,
            agentSessionId
          )] }
    ),
  ]
  const harness = createCodingAgentHarness({
    // Shell/reference contexts carry the session id (= conversation id);
    // session-less contexts get their own scratch namespace.
    host: (ctx): Promise<Host> => {
      if (!('agentSessionId' in ctx))
        throw new Error('Machine operations require a conversation')
      return targets.hostFor(ctx.agentSessionId)
    },
    commands: (ctx) => commandsFor(ctx.agentSessionId),
    context: switchAnnouncementPreamble(control, runnerRegistry),
  })

  const shellEnvironmentFor: ShellEnvironmentFactory = ctx => {
    sessionCommands.set(ctx.agentSessionId, {
      rootSessionId: ctx.rootSessionId,
      commands: ctx.commands
    })
    return createShellEnvironment({
      ...ctx,
      runJob: browsers ? (signal, operation) => {
        if (!(ctx.host instanceof RemoteHost))
          throw new Error('Browser jobs require a runner Host')
        return browsers.run(ctx.rootSessionId, ctx.host, signal, operation)
      } : undefined,
      retainEdits: (commandId, files) => changes.retain(ctx.rootSessionId, commandId, ctx.host, files),
    })
  }

  const agentServer = new AgentServer({
    agent: harness,
    providers: resolveProvider,
    shellEnvironment: shellEnvironmentFor,
    // A conversation's session tree — its root and every subagent — persists
    // as node and block rows in the conversation's database.
    store: (conversationId) => conversationStores.treeStore(conversationId),
  })

  const { upgradeWebSocket, websocket } = createBunWebSocket()
  const runnerReleaseDir = options.runnerReleaseDir ??
    process.env.DEMI_RUNNER_RELEASE_DIR
  const conversationForks = new ConversationForks({
    changes, control, stores: conversationStores, server: agentServer, registry: runnerRegistry,
  })
  await conversationForks.recover()

  const app = createApp({
    webDirectory: options.webDirectory,
    ...(runnerReleaseDir ? { runnerInstallation: {
        directory: runnerReleaseDir,
        backendUrl: options.publicUrl
      } } : {}),
    control,
    conversationStores,
    productState: new ProductState({
      control,
      stores: conversationStores,
      server: agentServer,
      registry: runnerRegistry,
      vault,
      assembly,
      managed: managedHosts,
      mode: options.mode
    }),
    conversationForks,
    conversationUpdates: new ConversationUpdates({
      control,
      vault,
      mode: options.mode,
      targets,
      agentServer
    }),
    admitFrame: (id) => targets.files(id).tryEnter(),
    vault,
    assembly,
    vendors,
    logins,
    providerOperations,
    providerAccounts: new ProviderAccounts(vault, assembly),
    agentServer,
    runnerRegistry,
    pipes,
    upgradeWebSocket,
    blobs,
    changes,
    withHost: (id, operation, options) => targets.withHost(id, operation, options),
    managedHosts,
    createCloudWorkspace: managedHosts
      ? (userId, name) => createCloudWorkspace({
        control,
        managedHosts,
        registry: runnerRegistry
      }, userId, name)
      : null,
    sessions,
    loginLimiter,
    emailChanges,
    mode: options.mode,
  })

  const server = Bun.serve({
    port: options.port ?? 0,
    idleTimeout: 0,
    fetch: app.fetch,
    websocket,
  })
  const url = `http://localhost:${server.port}`

  return {
    port: server.port ?? 0,
    url,
    managedHosts,
    close: async () => {
      const cleanup = new AsyncDisposableStack()
      cleanup.defer(() => controlDb.close())
      cleanup.defer(() => assembly.close())
      cleanup.defer(() => conversationStores.close())
      cleanup.defer(() => { server.stop(true) })
      cleanup.defer(() => runnerRegistry.close())
      cleanup.defer(() => pipes.close())
      cleanup.defer(() => managedHosts?.close())
      cleanup.defer(() => browsers?.close())
      cleanup.defer(() => agentServer.close())
      cleanup.defer(() => lifecycle.close())
      cleanup.defer(() => logins.close())
      await cleanup.disposeAsync()
    },
  }
}
