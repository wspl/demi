import { join } from 'node:path'
import { AgentServer, injectSubagentCommand, subagentCommandShape, type ProviderResolver, type ShellEnvironmentFactory } from '@demicodes/agent'
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
import { connectionOwner, ownerFitsMode } from './vault/scope'
import { WebSessions, type WebSessionsOptions } from './auth/sessions'
import { switchAnnouncementPreamble } from './conversation/switch-announcement'
import { createVirtualHostFactory } from './conversation/virtual-hosts'
import { createHostlessShell, transpileCommandModule } from './conversation/hostless-shell'
import { UpgradingShell, type Machine } from './conversation/upgrading-shell'
import { resolveExecutionTarget } from './conversation/execution-target'
import { HOSTLESS_HOME } from './conversation/scoped-transport'
import { createCloudWorkspace } from './managed/cloud-workspace'
import { createHostCommandGroup } from './managed/host-command'
import { ManagedHostError, ManagedHosts, ownerOf, type ManagedHostsConfig } from './managed/lifecycle'
import type { ManagedHostProvisioner } from './managed/provisioner'
import { createApp } from './http/app'
import { ProviderAssembly, builtinProviderTypes, usageAppender, type ProviderTypeFactory } from './llm/assembly'
import { meterProvider } from './llm/metering'
import { RunnerRegistry, type RunnerRegistryOptions } from './runner/registry'
import { TransferBroker } from './runner/transfers'
import { ProviderRateLimiter } from './usage/rate-limit'
import { ConnectionVault } from './vault/connections'
import { loadOrCreateInstanceSecret } from './vault/secret'
import { SubscriptionLoginFlows } from './vault/subscription-login'
import { DirBlobStore } from './storage/blob-store'
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
  /** Extra provider-type factories merged over the builtins — tests register stubs here. */
  providerTypes?: Record<string, ProviderTypeFactory>
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
  // The mode is fixed for the instance's life: connections configured under
  // the other mode would change owner meaning, so the process refuses to start.
  const misfits = (await control.listConnections('all')).filter((row) => !ownerFitsMode(options.mode, row.ownerUserId))
  if (misfits.length > 0) {
    controlDb.close()
    throw new Error(`${misfits.length} connection(s) were configured under the other instance mode; the mode cannot change once providers are configured`)
  }
  const sessions = new WebSessions(control, options.auth)
  const loginLimiter = new LoginLimiter(options.auth)

  const blobs = new DirBlobStore(join(options.dataDir, 'blobs'))
  const conversationStores = new ConversationStores(join(options.dataDir, 'conversations'), blobs)
  const virtualHostFor = createVirtualHostFactory({ conversationStores })

  // The command tree, defined once: the manifest every runner caches is built
  // from it plus the shape of the `agent` node every session grafts on. An
  // rpc command a runner relays runs as the session the job's environment
  // names — a conversation or a subagent — against the tree and the Host its
  // shell was built with.
  let manifest: Promise<Manifest> | null = null
  const sessionShells = new Map<string, { host: Host; commands: CommandRegistry }>()
  const transfers = new TransferBroker()
  const runnerRegistry = new RunnerRegistry({
    control,
    transfers,
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
    rpc: async (call, io) => {
      const shell = sessionShells.get(call.agentSessionId)
      if (!shell) throw new Error(`no live session ${call.agentSessionId} behind this job`)
      const transport = inProcessRpc(shell.commands.list(), {
        storage: new AgentSessionCommandStorage(shell.host.store, call.agentSessionId),
        host: shell.host,
      })
      const result = await transport({
        root: call.root,
        path: call.path,
        argv: call.argv,
        args: call.args,
        json: call.json,
        stdin: call.stdin,
        cwd: call.cwd,
        env: call.env,
        // The relayed io carries the calling device as a transfer destination
        // (`host shell --id` streams its stdout there, not over the sockets).
        io: { stdout: (data) => io.stdout(toBytes(data)), stderr: (data) => io.stderr(toBytes(data)), transferDestination: io.transferDestination } as CommandIO,
        signal: new AbortController().signal,
        stdinStream: io.stdinStream,
      })
      return result.exitCode
    },
    ...options.runner,
  })

  const vault = new ConnectionVault(control, loadOrCreateInstanceSecret(options.dataDir))
  const vaultRoot = join(options.dataDir, 'vault')
  const assembly = new ProviderAssembly(vault, { ...builtinProviderTypes(), ...options.providerTypes }, vaultRoot)
  const logins = new SubscriptionLoginFlows(vault, assembly, { vaultRoot })
  const rateLimiter = new ProviderRateLimiter(options.usage?.providerRequestsPerMinute)

  // connectionId = providerId: the LLM module assembles the connection's base
  // provider from vault credentials and wraps it with metering + enforcement
  // in the session's user/conversation context. Providers whose transport
  // runs on the execution target (requiresProcessCapableHost) get a
  // session-scoped instance carrying the target's spawn; the provider itself
  // resolves and injects its credential at spawn time.
  const resolveProvider: ProviderResolver = async (providerId, { agentSessionId }) => {
    let resolved = await assembly.providerFor(providerId)
    if (!resolved) return null
    if (resolved.provider.requiresProcessCapableHost) {
      const host = await hostFor(agentSessionId)
      resolved = await assembly.providerFor(providerId, {
        spawn: (params) => {
          const spawn = host.process.spawn
          if (!spawn) throw new Error('this provider needs a machine: the conversation runs hostless')
          return spawn.call(host.process, params)
        },
      })
      if (!resolved) return null
    }
    const conversation = await control.getConversation(agentSessionId)
    if (!conversation) throw new Error(`no conversation ${agentSessionId} behind this session`)
    const userId = conversation.userId
    // A connection outside the user's scope is unknown to the session.
    if (resolved.connection.ownerUserId !== connectionOwner(options.mode, userId)) return null
    return meterProvider(resolved.provider, {
      observe: usageAppender(control, { userId, conversationId: agentSessionId, connectionId: providerId }),
      beforeRequest: () => rateLimiter.take(userId),
    })
  }

  // Whether the owner of a managed host has a turn in flight: the idle rule's
  // first input. A workspace host counts turns across all its conversations.
  const turnInFlight = async (owner: ManagedHostOwner): Promise<boolean> => {
    const ids = owner.kind === 'conversation' ? [owner.id] : await control.listConversationIdsInWorkspace(owner.id)
    return ids.some((id) => {
      const phase = agentServer.sessionPhase(id)
      return phase !== null && phase !== 'idle'
    })
  }
  const managedHosts = options.managedHosts
    ? new ManagedHosts({
        control,
        registry: runnerRegistry,
        provisioner: options.managedHosts.provisioner,
        config: options.managedHosts.config,
        backendUrl: () => options.publicUrl ?? url,
        turnInFlight,
      })
    : null

  // The execution target is resolved server-side from the conversation record
  // (`sessions-and-targets.md` § The three states): a workspace or a
  // session-bound managed host routes to the device's stable RemoteHost
  // (offline ⇒ tool errors until the runner reattaches), neither ⇒ virtual.
  // A managed device is woken here — this is "the next action needing the
  // host" — after the owner check every control-plane path makes.
  const hostFor = async (conversationId: string): Promise<Host> => {
    const conversation = await control.getConversation(conversationId)
    if (!conversation) return virtualHostFor(conversationId)
    const target = await resolveExecutionTarget(control, conversation)
    if (target.kind === 'hostless') return virtualHostFor(conversationId)
    const device = await control.getDevice(target.deviceId)
    if (device?.kind === 'managed') {
      const owner = ownerOf(device)
      const owned = target.kind === 'workspace' ? owner.kind === 'workspace' && owner.id === target.workspaceId : owner.kind === 'conversation' && owner.id === conversationId
      if (!owned) throw new ManagedHostError('not_owner', `machine ${device.id} is bound to another owner`)
      if (!managedHosts) throw new Error('this backend provisions no machines')
      await managedHosts.ensureRunning(device)
    }
    const path = target.kind === 'workspace' ? target.path : (runnerRegistry.deviceIdentity(target.deviceId)?.homeDir ?? '/')
    return runnerRegistry.hostFor({ deviceId: target.deviceId, path }, conversationId, conversationStores.hostStore(conversationId))
  }

  const hostCommandDeps = {
    control,
    registry: runnerRegistry,
    transfers,
    managedHosts,
    virtualHostFor: (conversationId: string): Promise<Host> => virtualHostFor(conversationId),
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
    preamble: switchAnnouncementPreamble(control, runnerRegistry),
  })
  // The session upgrade (`sessions-and-targets.md` § Hostless execution):
  // the conversation's files become the home of a machine provisioned for
  // it, the conversation is bound to it silently, and the shell the first
  // script runs in is the one the session uses from then on.
  const upgradeToMachine = async (conversationId: string, ctx: Parameters<ShellEnvironmentFactory>[0]): Promise<Machine> => {
    if (!managedHosts) throw new Error('this script needs a machine, and this backend provisions none')
    const conversation = await control.getConversation(conversationId)
    if (!conversation) throw new Error(`no conversation ${conversationId}`)
    const home = join(options.dataDir, 'staging', conversationId)
    await conversationStores.materializeFiles(conversationId, [
      { from: HOSTLESS_HOME, to: home },
      { from: '/tmp', to: join(home, '.tmp') },
    ])
    const device = await managedHosts.provision({ kind: 'conversation', id: conversationId }, conversation.userId, home)
    if (!(await control.bindConversationHost(conversationId, device.id))) throw new Error('the conversation left the hostless state meanwhile')
    conversationStores.clearFiles(conversationId)
    const host = await hostFor(conversationId)
    return { environment: await shellEnvironmentFor({ ...ctx, host }), home: host.identity.homeDir }
  }

  // One shell environment per (session, Host), whichever side asks first:
  // the session through the server, or the upgrade for the machine it just
  // bound. A hostless conversation's shell is tinybash over its files tree
  // behind the upgrade; a machine's is the runner's job table.
  const shellEnvironments = new Map<string, WeakMap<Host, Promise<ShellEnvironment>>>()
  const shellEnvironmentFor = (ctx: Parameters<ShellEnvironmentFactory>[0]): Promise<ShellEnvironment> => {
    let bySession = shellEnvironments.get(ctx.agentSessionId)
    if (!bySession) {
      bySession = new WeakMap()
      shellEnvironments.set(ctx.agentSessionId, bySession)
    }
    let environment = bySession.get(ctx.host)
    if (!environment) {
      environment = (async () => {
        sessionShells.set(ctx.agentSessionId, { host: ctx.host, commands: ctx.commands })
        if (ctx.host instanceof VirtualHost) return new UpgradingShell(await createHostlessShell(ctx), () => upgradeToMachine(ctx.agentSessionId, ctx), HOSTLESS_HOME)
        if (ctx.host instanceof RemoteHost) return new RemoteShellEnvironment({ ...ctx.shell, host: ctx.host })
        throw new Error('the backend runs conversations hostless or through a runner; no other Host exists')
      })()
      bySession.set(ctx.host, environment)
    }
    return environment
  }

  const agentServer = new AgentServer({
    agent: harness,
    providers: resolveProvider,
    shell: { initialEnv: { PATH: '/usr/bin:/bin' } },
    shellEnvironment: shellEnvironmentFor,
    // Sessions persist as block rows in their conversation database; the
    // Host-store default never runs in the product backend.
    sessionStore: (agentSessionId) => conversationStores.sessionStore(agentSessionId),
    blobs,
  })

  const { upgradeWebSocket, websocket } = createBunWebSocket()
  const app = createApp({
    control,
    conversationStores,
    vault,
    assembly,
    logins,
    agentServer,
    runnerRegistry,
    transfers,
    upgradeWebSocket,
    blobs,
    hostFor,
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
      transfers.close()
      await runnerRegistry.close()
      server.stop(true)
      conversationStores.close()
      controlDb.close()
    },
  }
}
