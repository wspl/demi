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
import { createRemoteShellEnvironmentFactory, type RemoteShellEnvironmentFactoryOptions } from '@demicodes/host-remote'
import { CONVERSATION_IDLE_MS, LifecycleCoordinator } from './lifecycle/coordinator'
import { ConversationLifecycle } from './lifecycle/conversations'
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
import { buildCommandContext } from './runner/command-context'
import { createHostCommandGroup } from './runner/host-command'
import { Exposes } from './expose/records'
import { ExposeRelay } from './expose/relay'
import { ManagedHosts, type ManagedHostsConfig } from './managed/lifecycle'
import type { ManagedHostProvisioner } from '@demicodes/machines'
import { createApp } from './http/app'
import { withDrain } from './http/user-streams'
import type { NativeBinding } from '@demicodes/command-protocol'
import {
  ProviderAssembly,
  builtinProviderTypes,
  type ProviderType
} from './llm/assembly'
import { VendorCatalog } from './llm/vendors'
import type { ModelsDevFetch } from '@demicodes/provider'
import { createSessionProviderResolver } from './llm/session-providers'
import { ConversationTitles } from './conversation/title'
import { RunnerRegistry, type RunnerRegistryOptions } from './runner/registry'
import { PipeBroker } from '@demicodes/host-remote'
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
  /**
   * Generated conversation titles (`product.md` § Conversation titles). On by
   * default; off leaves every title as the first message gave it, which is
   * what a caller counting provider requests wants.
   */
  conversationTitles?: boolean
  /** Published historical edits; the caller owns an injected storage client. */
  changeObjects?: ChangeObjects
  /**
   * `DEMI_EXPOSE_DOMAIN` (`expose.md` § Deployment): the domain under which
   * expose hostnames live. Unset disables the feature everywhere.
   */
  exposeDomain?: string
  /** Exact native package catalog and deployment-owned artifact resolution. */
  nativeCommands: RemoteShellEnvironmentFactoryOptions
  /**
   * The user streams a page may open, by name, each bound to an operation
   * of a package in `nativeCommands` (`native-runtime.md` § User streams).
   * By default the live browser view, when the package provides it.
   */
  userStreams?: Record<string, NativeBinding>
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
  runner?: Omit<RunnerRegistryOptions, 'control' | 'pipes'>
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
  /** One conversation idle window, shared by paired-device release and Cloud stop. */
  lifecycle?: { idleMs?: number; now?: () => number; pollMs?: number }
  /** Usage-enforcement tuning — tests only. */
  usage?: { providerRequestsPerMinute?: number }
  /** Expose timing tuning — tests only. */
  expose?: { now?: () => number; sweepMs?: number; idleTimeoutMs?: number }
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
  const idleMs = options.lifecycle?.idleMs ?? CONVERSATION_IDLE_MS
  const lifecycle = new LifecycleCoordinator(options.lifecycle?.now)
  const createShellEnvironment = createRemoteShellEnvironmentFactory(options.nativeCommands)
  const browserPackage = options.nativeCommands.packages.find(descriptor => descriptor.id === 'demi.builtin' && descriptor.operations.includes('browser.open'))
  // Declared with the command tree, fixed for the program's lifetime; a
  // binding the packages do not provide declares nothing.
  const userStreams = new Map(Object.entries(options.userStreams ?? {
    browser: { package: 'demi.builtin', operation: 'browser.live' },
  }).flatMap(([name, binding]) => {
    const descriptor = options.nativeCommands.packages.find(candidate =>
      candidate.id === binding.package && candidate.operations.includes(binding.operation))
    return descriptor
      ? [[name, {
          package: descriptor,
          operation: binding.operation,
          resolveArtifact: options.nativeCommands.resolveArtifact,
        }] as const]
      : []
  }))
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
      const { caller } = execution.context
      const shell = caller.kind === 'agent' ? sessionCommands.get(caller.node) : undefined
      if (!shell || shell.rootSessionId !== execution.conversationId)
        throw new Error(`no authorized session behind job ${call.jobId}`)
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
        env: call.env,
        context: execution.context,
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
  })
  // The expose records (`expose.md`): one-hour public URLs over the relay.
  // Without a domain the service still answers — `add` and the product
  // surface say the feature is unavailable.
  const exposes = new Exposes({
    control,
    domain: options.exposeDomain ?? null,
    deviceOnline: deviceId => runnerRegistry.deviceOnline(deviceId),
    now: options.expose?.now ?? (() => Date.now()),
    sweepMs: options.expose?.sweepMs,
    origin: () => options.publicUrl ?? url,
  })
  const managedHosts = options.managedHosts
    ? new ManagedHosts({
      lifecycle,
      idleMs,
      now: options.lifecycle?.now,
      onLeftRunning: deviceId => exposes.destroyForDevice(deviceId),
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

  const conversations = new ConversationLifecycle({
    lifecycle,
    control,
    registry: runnerRegistry,
    targets: () => targets,
    idleMs,
    idlePollMs: options.lifecycle?.pollMs,
    treeActive: id => agentServer.treeActive(id),
    observeTree: (id, changed) => agentServer.observeTreeActivity(id, changed),
    reserveTree: id => agentServer.reserveTreeMutation(id),
  })

  // One target resolver serves the root and all descendants.
  const targets = new ConversationTargets({
    lifecycle: conversations,
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
    withHost: targets.withHost.bind(targets),
    exposes,
  }
  const commandsFor = (agentSessionId: string): Command[] => [
    createDemiCommand(
      { browser: browserPackage !== undefined, extraSubcommands: [createHostCommandGroup(
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
      commandContext: () => buildCommandContext(
        control,
        ctx.rootSessionId,
        { kind: 'agent', node: ctx.agentSessionId }
      ),
      runJob: (signal, operation) => targets.withHost(ctx.rootSessionId, async host => {
        if (host !== ctx.host) throw new Error('Conversation Host changed before job dispatch')
        return operation()
      }, { signal }),
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
  // The public relay exists only with a domain: an expose hostname is never
  // answered by the product routes, and without the domain there are none.
  const relay = options.exposeDomain
    ? new ExposeRelay({
      exposes,
      targets,
      pipes,
      upgradeWebSocket,
      idleTimeoutMs: options.expose?.idleTimeoutMs,
    })
    : null
  const runnerReleaseDir = options.runnerReleaseDir ??
    process.env.DEMI_RUNNER_RELEASE_DIR
  const conversationForks = new ConversationForks({
    changes, control, stores: conversationStores, server: agentServer, registry: runnerRegistry,
  })
  await conversationForks.recover()

  const titles = new ConversationTitles({
    control,
    resolveProvider,
    enabled: options.conversationTitles ?? true,
    log: console.warn,
  })

  const app = createApp({
    webDirectory: options.webDirectory,
    titles,
    ...(relay ? { relay } : {}),
    exposes,
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
      mode: options.mode,
      exposes,
    }),
    conversationForks,
    conversationUpdates: new ConversationUpdates({
      control,
      vault,
      mode: options.mode,
      targets,
      agentServer,
      titles
    }),
    // A transition holding the conversation is waited for, never refused
    // (`sessions-and-targets.md` § Coordinate shared Cloud activity).
    admitFrame: (id, signal) => targets.files(id).enter(signal),
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
    withHost: targets.withHost.bind(targets),
    transfer: targets.transfer.bind(targets),
    targets,
    userStreams,
    ...(options.publicUrl ? { publicOrigin: new URL(options.publicUrl).origin } : {}),
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
    // Bun's own cap would cut a streamed body short too, a download's pipe
    // or an upload; the routes cap what they read whole (`body-caps.ts`).
    maxRequestBodySize: Number.MAX_SAFE_INTEGER,
    fetch: app.fetch,
    websocket: withDrain(websocket),
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
      cleanup.defer(() => targets.close())
      cleanup.defer(() => conversations.close())
      cleanup.defer(() => agentServer.close())
      cleanup.defer(() => exposes.close())
      cleanup.defer(() => titles.close())
      cleanup.defer(() => lifecycle.close())
      cleanup.defer(() => logins.close())
      await cleanup.disposeAsync()
    },
  }
}
