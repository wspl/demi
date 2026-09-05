import { join } from 'node:path'
import { AgentServer, injectSubagentCommand, subagentCommandShape, type ShellEnvironmentFactory } from '@demicodes/agent'
import { createCodingAgentHarness, createDemiCommand } from '@demicodes/coding-agent'
import { VirtualHost } from '@demicodes/host-virtual'
import { buildManifest, inProcessRpc, type Manifest } from '@demicodes/command-loader'
import { RemoteHost, RemoteShellEnvironment } from '@demicodes/host-remote'
import { AgentSessionCommandStorage, type Command, type CommandIO, type CommandRegistry } from '@demicodes/shell'
import { toBytes } from '@demicodes/utils'
import type { Host, ShellEnvironment } from '@demicodes/shell'
import { createBunWebSocket } from 'hono/bun'
import type { InstanceMode } from './auth/identity'
import { LoginLimiter, type LoginLimiterOptions } from './auth/login-limiter'
import { ownerFitsMode } from './vault/scope'
import { WebSessions, type WebSessionsOptions } from './auth/sessions'
import { switchAnnouncementPreamble } from './conversation/switch-announcement'
import { createVirtualHostFactory } from './conversation/virtual-hosts'
import { HOSTLESS_ENV, createHostlessShell, transpileCommandModule } from './conversation/hostless-shell'
import { UpgradingShell } from './conversation/upgrading-shell'
import { ConversationTargets } from './conversation/target'
import { HOSTLESS_HOME } from './conversation/scoped-transport'
import { createCloudWorkspace } from './managed/cloud-workspace'
import { createHostCommandGroup } from './managed/host-command'
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
import { LocalControlService, type ControlService, type ManagedHostOwner } from './storage/control'
import { openSqliteDatabase } from './storage/database'
import { CONTROL_MIGRATIONS, migrate } from './storage/migrations'

export interface BackendOptions {
  /** Data directory: control database, conversation databases, blobs, virtual filesystems. */
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
   * sizes. A deployment requirement; a backend without it has no machine to
   * upgrade a hostless conversation to, and says so as an ordinary tool error.
   */
  managedHosts?: { provisioner: ManagedHostProvisioner; config?: Partial<ManagedHostsConfig> }
}

export interface Backend {
  port: number
  url: string
  /** The lifecycle, when configured. The product's triggers are the session upgrade and the Cloud workspace; tests drive it directly. */
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
  const virtualHostFor = createVirtualHostFactory({ conversationStores })

  // The command tree, defined once: the manifest every runner caches is built
  // from it plus the shape of the `agent` node every session grafts on. An
  // rpc command a runner relays runs as the session the job's environment
  // names — a conversation or a subagent — against the tree and the Host its
  // shell was built with.
  let manifest: Promise<Manifest> | null = null
  const sessionCommands = new Map<string, { rootSessionId: string; commands: CommandRegistry }>()
  const pipes = new PipeBroker()
  const runnerRegistry = new RunnerRegistry({
    control,
    pipes,
    // Bound late: the lifecycle is built over the registry below.
    homeGrow: async (deviceId, bytes) => {
      if (!managedHosts) throw new Error('this backend provisions no machines')
      await managedHosts.growHome(deviceId, bytes)
    },
    manifest: () =>
      (manifest ??= (async () => {
        const profiles = (await harness.agents?.({ state: harness.initialState(), cwd: HOSTLESS_HOME })) ?? []
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

  const vault = new ProviderVault(control, loadOrCreateInstanceSecret(options.dataDir))
  const vaultRoot = join(options.dataDir, 'vault')
  const vendors = new VendorCatalog(options.modelsDev ?? {})
  const assembly = new ProviderAssembly(vault, { ...builtinProviderTypes(), ...options.providerTypes }, vaultRoot, vendors)
  const logins = new SubscriptionLoginFlows(vault, assembly, { vaultRoot })
  const rateLimiter = new ProviderRateLimiter(options.usage?.providerRequestsPerMinute)

  const resolveProvider = createSessionProviderResolver({ assembly, control, mode: options.mode, hostFor: (id) => hostFor(id), rateLimiter })

  // Whether the owner of a managed host has a turn in flight: the idle rule's
  // first input. A workspace host counts turns across all its conversations.
  const turnInFlight = async (owner: ManagedHostOwner): Promise<boolean> => {
    const ids = owner.kind === 'conversation' ? [owner.id] : await control.listConversationIdsInWorkspace(owner.id)
    return ids.some((id) => agentServer.treeActive(id))
  }
  const managedHosts = options.managedHosts
    ? new ManagedHosts({
        control,
        registry: runnerRegistry,
        provisioner: options.managedHosts.provisioner,
        config: options.managedHosts.config,
        backendUrl: () => options.publicUrl ?? url,
        turnInFlight,
        reserveIdle: async owner => {
          const ids = owner.kind === 'conversation' ? [owner.id] : await control.listConversationIdsInWorkspace(owner.id)
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

  // Where a conversation's actions run (`sessions-and-targets.md`): one
  // module resolves the three states to a Host and moves between them — the
  // user's switch, the silent upgrade, the hostless re-entry rule.
  const targets = new ConversationTargets({
    control,
    registry: runnerRegistry,
    managedHosts,
    virtualHostFor,
    stores: conversationStores,
    stagingDir: join(options.dataDir, 'staging'),
    reserveTree: (conversationId) => agentServer.reserveTreeMutation(conversationId),
  })
  await targets.recoverUpgrades()
  const hostFor = (conversationId: string): Promise<Host> => targets.hostFor(conversationId)

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
    host: (ctx): Promise<Host> => ('agentSessionId' in ctx ? hostFor(ctx.agentSessionId) : virtualHostFor('lobby')),
    commands: (ctx) => commandsFor(ctx.agentSessionId),
    context: switchAnnouncementPreamble(control, runnerRegistry),
  })

  // The environment a shell starts with (`sessions-and-targets.md` § What
  // moves): the hostless table for the hostless shell and for a managed
  // host, so the two sides agree; nothing for a user host, whose jobs run
  // in the device user's own environment. The session's own entries (the
  // subagent ids) come on top.
  const shellOptionsFor = (ctx: Parameters<ShellEnvironmentFactory>[0]) => {
    const table = ctx.host instanceof VirtualHost || targets.hostInfo(ctx.host)?.device === 'managed' ? HOSTLESS_ENV : {}
    return { ...ctx.shell, initialEnv: { ...table, ...ctx.shell.initialEnv } }
  }

  // One shell environment per (session, Host). A hostless conversation's is
  // tinybash over its files tree behind the upgrade; a machine's is the
  // runner's job table. Across the upgrade they are one object: the
  // conversation's own machine is served by the session's hostless shell,
  // which adopts its shells there under the ids the model holds.
  const shellEnvironments = new Map<string, WeakMap<Host, Promise<ShellEnvironment>>>()
  const shellEnvironmentFor = (ctx: Parameters<ShellEnvironmentFactory>[0]): Promise<ShellEnvironment> => {
    let bySession = shellEnvironments.get(ctx.agentSessionId)
    if (!bySession) {
      bySession = new WeakMap()
      shellEnvironments.set(ctx.agentSessionId, bySession)
    }
    let environment = bySession.get(ctx.host)
    if (!environment) {
      const sessionHosts = bySession
      environment = (async (): Promise<ShellEnvironment> => {
        sessionCommands.set(ctx.agentSessionId, { rootSessionId: ctx.rootSessionId, commands: ctx.commands })
        const shell = shellOptionsFor(ctx)
        const info = targets.hostInfo(ctx.host)
        if (ctx.host instanceof VirtualHost) {
          // The conversation's, whichever session asks: a subagent's outside script upgrades its root's conversation.
          const conversationId = info?.conversationId ?? ctx.agentSessionId
          return new UpgradingShell(
            await createHostlessShell({ ...ctx, shell }),
            async () => {
              const machine = await targets.upgrade(conversationId)
              // Through this factory, so the session's environment for the machine is the hostless shell itself.
              await shellEnvironmentFor({ ...ctx, host: machine.host })
            },
            HOSTLESS_HOME,
            targets.files(conversationId),
            async () => {
              if ((await targets.resolve(conversationId)).kind !== 'hostless') {
                await shellEnvironmentFor({ ...ctx, host: await targets.hostFor(conversationId) })
              }
            },
          )
        }
        if (ctx.host instanceof RemoteHost) {
          const remote = new RemoteShellEnvironment({ ...shell, host: ctx.host })
          if (info?.target === 'host') {
            const hostless = await sessionHosts.get(await virtualHostFor(info.conversationId))
            if (hostless instanceof UpgradingShell) {
              hostless.attach({ environment: remote, home: ctx.host.identity.homeDir })
              return hostless
            }
          }
          return remote
        }
        throw new Error('the backend runs conversations hostless or through a runner; no other Host exists')
      })()
      bySession.set(ctx.host, environment)
    }
    return environment
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
  const app = createApp({
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
      ? (userId, name) => createCloudWorkspace({ control, managedHosts, registry: runnerRegistry, stagingDir: join(options.dataDir, 'staging') }, userId, name)
      : null,
    sessions,
    loginLimiter,
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
