import { join } from 'node:path'
import { AgentServer, injectSubagentCommand, subagentCommandShape, type ShellEnvironmentFactory } from '@demicodes/agent'
import { createCodingAgentHarness, createDemiCommand } from '@demicodes/coding-agent'
import { buildManifest, inProcessRpc, type Manifest } from '@demicodes/command-loader'
import { RemoteHost, RemoteShellEnvironment } from '@demicodes/host-remote'
import { AgentSessionCommandStorage, type Command, type CommandIO, type CommandRegistry } from '@demicodes/shell'
import { toBytes } from '@demicodes/utils'
import type { Host, ShellEnvironment } from '@demicodes/shell'
import { createBunWebSocket } from 'hono/bun'
import type { InstanceMode } from './auth/identity'
import { EmailChanges, type AccountMailSender } from './auth/email-change'
import { LoginLimiter, type LoginLimiterOptions } from './auth/login-limiter'
import { ownerFitsMode } from './vault/scope'
import { WebSessions, type WebSessionsOptions } from './auth/sessions'
import { switchAnnouncementPreamble } from './conversation/switch-announcement'
import { transpileCommandModule } from './conversation/command-manifest'
import { ConversationTargets } from './conversation/target'
import { CLOUD_HOME } from './conversation/execution-target'
import { createCloudWorkspace } from './managed/cloud-workspace'
import { createHostCommandGroup } from './runner/host-command'
import { ManagedHosts, type ManagedHostsConfig } from './managed/lifecycle'
import type { ManagedHostProvisioner } from './managed/provisioner'
import { createApp } from './http/app'
import { ProviderAssembly, builtinProviderTypes, type ProviderType } from './llm/assembly'
import { VendorCatalog } from './llm/vendors'
import type { ModelsDevFetch } from '@demicodes/provider'
import { createSessionProviderResolver } from './llm/session-providers'
import { RunnerRegistry, type RunnerRegistryOptions } from './runner/registry'
import { PipeBroker } from './runner/pipes'
import { ProviderRateLimiter } from './usage/rate-limit'
import { ProviderVault } from './vault/providers'
import { loadOrCreateInstanceSecret } from './vault/secret'
import { SubscriptionLoginFlows } from './vault/subscription-login'
import { UserBlobStores } from './storage/user-blobs'
import { ConversationStores } from './storage/conversation-store'
import { LocalControlService, type ControlService } from './storage/control'
import { openSqliteDatabase } from './storage/database'
import { CONTROL_MIGRATIONS, migrate } from './storage/migrations'

export interface BackendOptions {
  /** Directory produced by runner/runtime/release.ts; exposes paired client/runner downloads. */
  runnerReleaseDir?: string
  /** Delivers account email verification codes. */
  accountMail?: AccountMailSender
  /** Data directory: control database, conversation databases, blobs and machine images. */
  dataDir: string
  /** The instance mode, a deployment decision: `DEMI_INSTANCE_MODE`. */
  mode: InstanceMode
  /** HTTP port (0 = ephemeral, for tests). */
  port?: number
  /** The URL managed guests dial (`managed-hosts.md` § Network); default the local one, which only the fake's guests can reach. */
  publicUrl?: string
  /** Runner-management tuning (claim TTL, liveness interval) — tests only. */
  runner?: Omit<RunnerRegistryOptions, 'control'>
  /** Extra provider families merged over the builtins — tests register stubs here. */
  providerTypes?: Record<string, ProviderType>
  /** The models.dev fetch behind the vendor catalog — tests serve a fixture. */
  modelsDev?: { fetch?: ModelsDevFetch; url?: string }
  /** Usage-enforcement tuning — tests only. */
  usage?: { providerRequestsPerMinute?: number }
  /** Session lifetime and login lockout tuning — tests only. */
  auth?: WebSessionsOptions & LoginLimiterOptions
  /**
   * Managed hosts (`managed-hosts.md`): the provisioner and the lifecycle
   * sizes. Cloud operations require it; paired devices remain independently usable.
   */
  managedHosts?: { provisioner: ManagedHostProvisioner; config?: Partial<ManagedHostsConfig> }
}

export interface Backend {
  port: number
  url: string
  /** The lifecycle, when configured. Cloud operations allocate and wake machines on demand. */
  managedHosts: ManagedHosts | null
  close(): Promise<void>
}

/** The composition root: opens storage, assembles the services, mounts the HTTP surface. */
export async function createBackend(options: BackendOptions): Promise<Backend> {
  const controlDb = openSqliteDatabase(join(options.dataDir, 'control.sqlite'))
  migrate(controlDb, CONTROL_MIGRATIONS)
  const control: ControlService = new LocalControlService(controlDb)
  // The mode is fixed for the instance's life: providers configured under
  // the other mode would change owner meaning, so the process refuses to start.
  const misfits = (await control.listProviders('all')).filter((row) => !ownerFitsMode(options.mode, row.ownerUserId))
  if (misfits.length > 0) {
    controlDb.close()
    throw new Error(`${misfits.length} provider(s) were configured under the other instance mode; the mode cannot change once providers are configured`)
  }
  const sessions = new WebSessions(control, options.auth)
  const loginLimiter = new LoginLimiter(options.auth)

  const blobs = new UserBlobStores(join(options.dataDir, 'blobs'), control)
  const conversationStores = new ConversationStores(join(options.dataDir, 'conversations'), (id) => blobs.forConversation(id))

  // The command tree, defined once: the manifest every runner caches is built
  // from it plus the shape of the `agent` node every session grafts on. An
  // rpc command a runner relays runs as the session the job's environment
  // names — a conversation or a subagent — against the tree and the Host its
  // shell was built with.
  let manifest: Promise<Manifest> | null = null
  const sessionCommands = new Map<string, { rootSessionId: string; commands: CommandRegistry }>()
  const pipes = new PipeBroker()
  const runnerRegistry = new RunnerRegistry({
    admit: deviceId => managedHosts?.admit(deviceId) ?? (() => {}),
    control,
    pipes,
    // Bound late: the lifecycle is built over the registry below.
    volumeGrow: async (deviceId, volume, bytes) => {
      if (!managedHosts) throw new Error('this backend provisions no machines')
      await managedHosts.growVolume(deviceId, volume, bytes)
    },
    manifest: () =>
      (manifest ??= (async () => {
        const profiles = (await harness.agents?.({ state: harness.initialState(), cwd: CLOUD_HOME })) ?? []
        const roots = injectSubagentCommand(commandsFor(''), subagentCommandShape(profiles.map((profile) => profile.name)))
        return buildManifest(roots, { transpile: transpileCommandModule })
      })()),
    rpc: async (call, io, execution) => {
      const shell = sessionCommands.get(call.agentSessionId)
      if (!shell || shell.rootSessionId !== execution.conversationId) throw new Error(`no authorized session ${call.agentSessionId} behind this job`)
      const transport = inProcessRpc(shell.commands.list(), {
        storage: new AgentSessionCommandStorage(execution.host.store, call.agentSessionId),
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
        env: { ...call.env, DEMI_SESSION_ID: call.agentSessionId, DEMI_SHELL_ID: call.shellId },
        io: io.commandIO(),
        signal: io.signal,
        stdinStream: io.stdinStream,
      })
      return result.exitCode
    },
    ...options.runner,
  })

  const instanceSecret = loadOrCreateInstanceSecret(options.dataDir)
  const emailChanges = new EmailChanges(control, instanceSecret, options.accountMail, options.auth?.now)
  const vault = new ProviderVault(control, instanceSecret)
  const vaultRoot = join(options.dataDir, 'vault')
  const vendors = new VendorCatalog(options.modelsDev ?? {})
  const assembly = new ProviderAssembly(vault, { ...builtinProviderTypes(), ...options.providerTypes }, vaultRoot, vendors)
  const logins = new SubscriptionLoginFlows(vault, assembly, { vaultRoot })
  const rateLimiter = new ProviderRateLimiter(options.usage?.providerRequestsPerMinute)

  const resolveProvider = createSessionProviderResolver({ assembly, control, mode: options.mode, hostFor: (id) => targets.hostFor(id), rateLimiter })

  const cloudConversations = async (userId: string): Promise<string[]> => {
    const ids = await control.listUserConversationIds(userId)
    const device = await control.getManagedDevice(userId)
    const selected: string[] = []
    for (const id of ids) {
      const target = await targets.resolve(id)
      const attached = device ? await control.listAttachedHosts(id) : []
      if (target.kind === 'cloud' || target.deviceId === device?.id || attached.some(host => host.deviceId === device?.id)) selected.push(id)
    }
    return selected
  }
  const turnInFlight = async (userId: string): Promise<boolean> =>
    (await cloudConversations(userId)).some(id => agentServer.treeActive(id))
  const managedHosts = options.managedHosts
    ? new ManagedHosts({
        control,
        registry: runnerRegistry,
        provisioner: options.managedHosts.provisioner,
        config: options.managedHosts.config,
        backendUrl: () => options.publicUrl ?? url,
        turnInFlight,
        interrupt: async userId => {
          const releases: Array<() => void> = []
          try {
            for (const id of await cloudConversations(userId)) releases.push(await agentServer.interruptTree(id))
            return () => { for (const release of releases) release() }
          } catch (error) { for (const release of releases) release(); throw error }
        },
        reserveIdle: async userId => {
          const ids = await cloudConversations(userId)
          const releases: Array<() => void> = []
          for (const id of ids) {
            for (const reserve of [() => agentServer.reserveTreeMutation(id), () => targets.files(id).tryReserve()]) {
              const release = reserve()
              if (!release) { for (const held of releases) held(); return null }
              releases.push(release)
            }
          }
          return () => { for (const release of releases) release() }
        },
      })
    : null
  // Whatever a previous process left running or unsaved is settled before the first need can boot anything.
  await managedHosts?.reconcile()

  // One target resolver serves the root and all descendants.
  const targets = new ConversationTargets({
    control,
    registry: runnerRegistry,
    managedHosts,
    stores: conversationStores,
    reserveTree: (conversationId) => agentServer.reserveTreeMutation(conversationId),
  })

  const hostCommandDeps = {
    control,
    registry: runnerRegistry,
    pipes,
    managedHosts,
    hostStoreFor: (conversationId: string) => conversationStores.hostStore(conversationId),
  }
  const commandsFor = (agentSessionId: string): Command[] => [
    createDemiCommand({ extraSubcommands: [createHostCommandGroup(hostCommandDeps, agentSessionId)] }),
  ]
  const harness = createCodingAgentHarness({
    // Shell/reference contexts carry the session id (= conversation id);
    // session-less contexts get their own scratch namespace.
    host: (ctx): Promise<Host> => {
      if (!('agentSessionId' in ctx)) throw new Error('Machine operations require a conversation')
      return targets.hostFor(ctx.agentSessionId)
    },
    commands: (ctx) => commandsFor(ctx.agentSessionId),
    context: switchAnnouncementPreamble(control, runnerRegistry),
  })

  const shellEnvironmentFor = (ctx: Parameters<ShellEnvironmentFactory>[0]): ShellEnvironment => {
    sessionCommands.set(ctx.agentSessionId, { rootSessionId: ctx.rootSessionId, commands: ctx.commands })
    if (!(ctx.host instanceof RemoteHost)) throw new Error('The backend requires a runner Host')
    return new RemoteShellEnvironment({ ...ctx.shell, host: ctx.host })
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
  const runnerReleaseDir = options.runnerReleaseDir ?? process.env.DEMI_RUNNER_RELEASE_DIR
  const app = createApp({
    ...(runnerReleaseDir ? { runnerInstallation: { directory: runnerReleaseDir, backendUrl: options.publicUrl } } : {}),
    control,
    conversationStores,
    vault,
    assembly,
    vendors,
    logins,
    agentServer,
    runnerRegistry,
    pipes,
    upgradeWebSocket,
    blobs,
    withHost: (id, operation, signal) => targets.withHost(id, operation, signal),
    switchTarget: (conversationId, toWorkspaceId) => targets.switch(conversationId, toWorkspaceId),
    managedHosts,
    createCloudWorkspace: managedHosts
      ? (userId, name) => createCloudWorkspace({ control, managedHosts, registry: runnerRegistry }, userId, name)
      : null,
    sessions,
    loginLimiter,
    emailChanges,
    mode: options.mode,
  })

  const server = Bun.serve({
    port: options.port ?? 0,
    fetch: app.fetch,
    websocket,
  })
  const url = `http://localhost:${server.port}`

  return {
    port: server.port ?? 0,
    url,
    managedHosts,
    close: async () => {
      await managedHosts?.close()
      await agentServer.close()
      pipes.close()
      await runnerRegistry.close()
      server.stop(true)
      conversationStores.close()
      controlDb.close()
    },
  }
}
