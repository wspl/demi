import { resolve } from 'node:path'
import {
  createWebSocketServerTransport,
  type AgentServer,
  type AgentTransportBinding,
  type JsonWebSocket,
} from '@demicodes/agent'
import { createCodingAgentHarness } from '@demicodes/coding-agent'
import { LocalHost, createLocalAgentServer, type LocalAgentServerHandle } from '@demicodes/host-local'
import type { Provider } from '@demicodes/provider'
import type { BashEnvironmentOptions } from '@demicodes/shell'

export type AgentHubShellOptions = Omit<BashEnvironmentOptions, 'host' | 'commands'>

/** Holds one local AgentServer per workspace cwd (command bridge on by default). */
export class AgentHub {
  private readonly handles = new Map<string, LocalAgentServerHandle>()

  constructor(
    private readonly providers: Provider[],
    private readonly shellOptions: AgentHubShellOptions,
  ) {}

  attach(cwd: string, socket: JsonWebSocket): AgentTransportBinding {
    return this.serverFor(cwd).attachTransport(createWebSocketServerTransport(socket))
  }

  private serverFor(cwd: string): AgentServer {
    const key = resolve(cwd)
    const existing = this.handles.get(key)
    if (existing) return existing.server

    const host = new LocalHost(key)
    const harness = createCodingAgentHarness({ host })
    const handle = createLocalAgentServer({
      host,
      agent: harness,
      providers: this.providers,
      shell: this.shellOptions,
      // commandBridge defaults to true inside createLocalAgentServer
    })
    this.handles.set(key, handle)
    return handle.server
  }

  async close(): Promise<void> {
    const handles = [...this.handles.values()]
    this.handles.clear()
    await Promise.all(handles.map((handle) => handle.close()))
  }
}
