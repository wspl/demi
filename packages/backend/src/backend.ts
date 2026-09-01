import { join } from 'node:path'
import { AgentServer } from '@demicodes/agent'
import { createCodingAgentHarness } from '@demicodes/coding-agent'
import { LocalHost } from '@demicodes/host-local'
import type { Provider } from '@demicodes/provider'
import type { Host } from '@demicodes/shell'
import { createBunWebSocket } from 'hono/bun'
import { STUB_USER } from './auth/identity'
import { createVirtualHostFactory } from './conversation/virtual-hosts'
import { createApp } from './http/app'
import { DirBlobStore } from './storage/blob-store'
import { ConversationStores } from './storage/conversation-store'
import { LocalControlService, type ControlService } from './storage/control'
import { openSqliteDatabase } from './storage/database'
import { CONTROL_MIGRATIONS, migrate } from './storage/migrations'

export interface BackendOptions {
  /** Data directory: control database, conversation databases, blobs, virtual filesystems. */
  dataDir: string
  /** Operator-assembled providers (the vault arrives in M5). */
  providers: Provider[]
  /** HTTP port (0 = ephemeral, for tests). */
  port?: number
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

  const agentServer = new AgentServer({
    agent: createCodingAgentHarness({
      host: (ctx): Promise<Host> =>
        // Shell/reference contexts carry the session id (= conversation id);
        // session-less contexts get their own scratch namespace.
        virtualHostFor('agentSessionId' in ctx ? ctx.agentSessionId : 'lobby'),
    }),
    providers: options.providers,
    shell: { initialEnv: { PATH: '/usr/bin:/bin' } },
    // Sessions persist as block rows in their conversation database; the
    // Host-store default never runs in the product backend.
    sessionStore: (agentSessionId) => conversationStores.sessionStore(agentSessionId),
    blobs,
  })

  const { upgradeWebSocket, websocket } = createBunWebSocket()
  const app = createApp({
    control,
    conversationStores,
    providers: options.providers,
    agentServer,
    upgradeWebSocket,
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
      server.stop(true)
      conversationStores.close()
      controlDb.close()
    },
  }
}
