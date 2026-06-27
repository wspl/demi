import { resolve } from 'node:path'
import {
  AgentServer,
  createWebSocketServerTransport,
  type AgentTransportBinding,
  type JsonWebSocket,
} from '@demicodes/agent'
import { createCodingAgentHarness } from '@demicodes/coding-agent'
import { LocalHost } from '@demicodes/host-local'
import type { Provider } from '@demicodes/provider'
import type { BashEnvironmentOptions } from '@demicodes/shell'

export type AgentHubShellOptions = Omit<BashEnvironmentOptions, 'host' | 'commands'>

/** Holds one AgentServer per workspace cwd. Conversations in the same cwd share a server. */
export class AgentHub {
  private readonly servers = new Map<string, AgentServer>()

  constructor(
    private readonly providers: Provider[],
    private readonly shellOptions: AgentHubShellOptions,
  ) {}

  attach(cwd: string, socket: JsonWebSocket): AgentTransportBinding {
    return this.serverFor(cwd).attachTransport(createWebSocketServerTransport(socket))
  }

  private serverFor(cwd: string): AgentServer {
    const key = resolve(cwd)
    const existing = this.servers.get(key)
    if (existing) return existing
    const host = new LocalHost(key)
    const harness = createCodingAgentHarness({ host })
    const server = new AgentServer({ agent: harness, providers: this.providers, shell: this.shellOptions })
    this.servers.set(key, server)
    return server
  }

  async close(): Promise<void> {
    const servers = [...this.servers.values()]
    this.servers.clear()
    await Promise.all(servers.map((server) => server.close()))
  }
}
