import { resolve } from 'node:path'
import {
  AgentServer,
  createWebSocketServerTransport,
  type AgentTransportBinding,
  type JsonWebSocket,
} from '@demi/agent'
import { createCodingAgentHarness } from '@demi/coding-agent'
import { LocalHost } from '@demi/host-local'
import type { ProviderRegistry } from '@demi/provider'
import type { BashEnvironmentOptions } from '@demi/shell'

export type AgentHubShellOptions = Omit<BashEnvironmentOptions, 'host' | 'commands'>

/** Holds one AgentServer per workspace cwd. Conversations in the same cwd share a server. */
export class AgentHub {
  private readonly servers = new Map<string, AgentServer>()

  constructor(
    private readonly registry: ProviderRegistry,
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
    const server = new AgentServer({ agent: harness, providerRegistry: this.registry, shell: this.shellOptions })
    this.servers.set(key, server)
    return server
  }

  async close(): Promise<void> {
    const servers = [...this.servers.values()]
    this.servers.clear()
    await Promise.all(servers.map((server) => server.close()))
  }
}
