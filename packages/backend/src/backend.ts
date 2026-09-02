import { join } from 'node:path'
import { AgentServer, type ProviderResolver, defaultShellEnvironment } from '@demicodes/agent'
import { createCodingAgentHarness, createDemiCommand } from '@demicodes/coding-agent'
import { LocalHost } from '@demicodes/host-local'
import { VirtualHost } from '@demicodes/host-virtual'
import { buildManifest, inProcessRpc, type Manifest } from '@demicodes/command-loader'
import { RemoteHost, RemoteShellEnvironment } from '@demicodes/runner-protocol'
import { AgentSessionCommandStorage, type Command, type CommandIO } from '@demicodes/shell'
import { toBytes } from '@demicodes/utils'
import type { Host } from '@demicodes/shell'
import { createBunWebSocket } from 'hono/bun'
import { STUB_USER } from './auth/identity'
import { switchAnnouncementPreamble } from './conversation/switch-announcement'
import { createVirtualHostFactory } from './conversation/virtual-hosts'
import { createHostlessShell, transpileCommandModule } from './conversation/hostless-shell'
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
    localFs: new LocalHost(options.dataDir).fs,
  })

  // The command tree, defined once: the manifest every runner caches is built
  // from it, and an rpc command a runner relays runs against the tree of the
  // conversation the ids in the job's environment name.
  let manifest: Promise<Manifest> | null = null
  const transfers = new TransferBroker()
  const runnerRegistry = new RunnerRegistry({
    control,
    transfers,
    manifest: () => (manifest ??= buildManifest(commandsFor(''), { transpile: transpileCommandModule })),
    rpc: async (call, io) => {
      const host = await hostFor(call.agentSessionId)
      const transport = inProcessRpc(commandsFor(call.agentSessionId), {
        storage: new AgentSessionCommandStorage(host.store, call.agentSessionId),
        host,
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
        spawn: (params) => host.process.spawn(params),
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

  // The execution target is resolved server-side from the conversation record:
  // a workspace pointer routes to the device's stable RemoteHost (offline ⇒
  // tool errors until the runner reattaches), no workspace ⇒ virtual.
  const hostFor = async (conversationId: string): Promise<Host> => {
    const conversation = await control.getConversation(conversationId)
    const workspace = conversation?.workspaceId ? await control.getWorkspace(conversation.workspaceId) : null
    if (workspace) return runnerRegistry.hostFor(workspace, conversationId, conversationStores.hostStore(conversationId))
    return virtualHostFor(conversationId)
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
  const agentServer = new AgentServer({
    agent: createCodingAgentHarness({
      // Shell/reference contexts carry the session id (= conversation id);
      // session-less contexts get their own scratch namespace.
      host: (ctx): Promise<Host> => ('agentSessionId' in ctx ? hostFor(ctx.agentSessionId) : virtualHostFor('lobby')),
      commands: (ctx) => commandsFor(ctx.agentSessionId),
      preamble: switchAnnouncementPreamble(control),
    }),
    providers: resolveProvider,
    shell: { initialEnv: { PATH: '/usr/bin:/bin' } },
    // A hostless conversation's shell is tinybash over its store-backed Host;
    // a real host's is the runner's job table; anything else keeps just-bash.
    shellEnvironment: (ctx) =>
      ctx.host instanceof VirtualHost
        ? createHostlessShell(ctx)
        : ctx.host instanceof RemoteHost
          ? new RemoteShellEnvironment({ ...ctx.shell, host: ctx.host })
          : defaultShellEnvironment(ctx),
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
