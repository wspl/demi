import { join } from 'node:path'
import { AgentServer, injectSubagentCommand, subagentCommandShape, type ProviderResolver } from '@demicodes/agent'
import { createCodingAgentHarness, createDemiCommand } from '@demicodes/coding-agent'
import { nodeFileSystem } from '@demicodes/host-virtual/node'
import { VirtualHost } from '@demicodes/host-virtual'
import { buildManifest, inProcessRpc, type Manifest } from '@demicodes/command-loader'
import { RemoteHost, RemoteShellEnvironment } from '@demicodes/host-remote'
import { AgentSessionCommandStorage, type Command, type CommandIO, type CommandRegistry } from '@demicodes/shell'
import { toBytes } from '@demicodes/utils'
import type { Host } from '@demicodes/shell'
import { createBunWebSocket } from 'hono/bun'
import { STUB_USER } from './auth/identity'
import { switchAnnouncementPreamble } from './conversation/switch-announcement'
import { createVirtualHostFactory } from './conversation/virtual-hosts'
import { createHostlessShell, transpileCommandModule } from './conversation/hostless-shell'
import { resolveExecutionTarget } from './conversation/execution-target'
import { HOSTLESS_HOME } from './conversation/scoped-transport'
import { createHostCommandGroup } from './managed/host-command'
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
import { LocalControlService, type ControlService } from './storage/control'
import { openSqliteDatabase } from './storage/database'
import { CONTROL_MIGRATIONS, migrate } from './storage/migrations'

export interface BackendOptions {
  /** Data directory: control database, conversation databases, blobs, virtual filesystems. */
  dataDir: string
  /** HTTP port (0 = ephemeral, for tests). */
  port?: number
  /** Runner-management tuning (claim TTL, liveness interval) — tests only. */
  runner?: Omit<RunnerRegistryOptions, 'control'>
  /** Extra provider-type factories merged over the builtins — tests register stubs here. */
  providerTypes?: Record<string, ProviderTypeFactory>
  /** Usage-enforcement tuning — tests only. */
  usage?: { providerRequestsPerMinute?: number }
}

export interface Backend {
  port: number
  url: string
  close(): Promise<void>
}

/** The composition root: opens storage, assembles the services, mounts the HTTP surface. */
export async function createBackend(options: BackendOptions): Promise<Backend> {
  const controlDb = openSqliteDatabase(join(options.dataDir, 'control.sqlite'))
  migrate(controlDb, CONTROL_MIGRATIONS)
  const control: ControlService = new LocalControlService(controlDb)
  await control.ensureUser(STUB_USER)

  const blobs = new DirBlobStore(join(options.dataDir, 'blobs'))
  const conversationStores = new ConversationStores(join(options.dataDir, 'conversations'), blobs)
  const virtualHostFor = createVirtualHostFactory({
    dataDir: options.dataDir,
    conversationStores,
    localFs: nodeFileSystem(options.dataDir),
  })

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
  const logins = new SubscriptionLoginFlows(vault, assembly, { ownerUserId: STUB_USER.id, vaultRoot })
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
    const userId = conversation?.userId ?? STUB_USER.id
    return meterProvider(resolved.provider, {
      observe: usageAppender(control, { userId, conversationId: agentSessionId, connectionId: providerId }),
      beforeRequest: () => rateLimiter.take(userId),
    })
  }

  // The execution target is resolved server-side from the conversation record
  // (`sessions-and-targets.md` § The three states): a workspace or a
  // session-bound managed host routes to the device's stable RemoteHost
  // (offline ⇒ tool errors until the runner reattaches), neither ⇒ virtual.
  const hostFor = async (conversationId: string): Promise<Host> => {
    const conversation = await control.getConversation(conversationId)
    if (!conversation) return virtualHostFor(conversationId)
    const target = await resolveExecutionTarget(control, conversation)
    if (target.kind === 'hostless') return virtualHostFor(conversationId)
    const path = target.kind === 'workspace' ? target.path : (runnerRegistry.deviceIdentity(target.deviceId)?.homeDir ?? '/')
    return runnerRegistry.hostFor({ deviceId: target.deviceId, path }, conversationId, conversationStores.hostStore(conversationId))
  }

  const hostCommandDeps = {
    control,
    registry: runnerRegistry,
    transfers,
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
  const agentServer = new AgentServer({
    agent: harness,
    providers: resolveProvider,
    shell: { initialEnv: { PATH: '/usr/bin:/bin' } },
    // A hostless conversation's shell is tinybash over its store-backed Host;
    // a real host's is the runner's job table; no other Host exists.
    shellEnvironment: (ctx) => {
      sessionShells.set(ctx.agentSessionId, { host: ctx.host, commands: ctx.commands })
      if (ctx.host instanceof VirtualHost) return createHostlessShell(ctx)
      if (ctx.host instanceof RemoteHost) return new RemoteShellEnvironment({ ...ctx.shell, host: ctx.host })
      throw new Error('the backend runs conversations hostless or through a runner; no other Host exists')
    },
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
  })

  const server = Bun.serve({
    port: options.port ?? 0,
    fetch: app.fetch,
    websocket,
  })

  return {
    port: server.port ?? 0,
    url: `http://localhost:${server.port}`,
    close: async () => {
      await agentServer.close()
      transfers.close()
      await runnerRegistry.close()
      server.stop(true)
      conversationStores.close()
      controlDb.close()
    },
  }
}
