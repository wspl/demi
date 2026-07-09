import { mkdirSync } from 'node:fs'
import { join } from 'node:path'
import {
  AgentServer,
  type AgentHarness,
  type AgentServerOptions,
  type AgentServerSessionOptions,
} from '@demicodes/agent'
import { COMMAND_BRIDGE_SHIM_SOURCE, startCommandBridge, type CommandBridgeHandle } from '@demicodes/agent/command-bridge'
import type { Provider } from '@demicodes/provider'
import type { BashEnvironmentOptions } from '@demicodes/shell'
import type { LocalHost } from './local-host'

export interface CreateLocalAgentServerOptions {
  /** Same LocalHost instance the harness was built with. */
  host: LocalHost
  agent: AgentHarness<unknown>
  providers: Provider[]
  shell?: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  session?: AgentServerSessionOptions
  /**
   * Command bridge for real OS subprocesses. **Default true** (open box).
   * Pass `false` only to disable.
   */
  commandBridge?: boolean
  /**
   * Override UDS path when bridge is enabled.
   * Default: `<host.defaultCwd>/.demi/command-bridge.sock` (short for macOS AF_UNIX limits).
   */
  commandBridgeSocketPath?: string
}

export interface LocalAgentServerHandle {
  server: AgentServer
  host: LocalHost
  /** Stops the bridge listener (if any) then closes the server. */
  close(): Promise<void>
}

/**
 * Open-box local agent assembly: `LocalHost` + `AgentServer` with command
 * bridge **on by default**. Products should use this instead of hand-wiring
 * socket paths, shim source, and the UDS listener.
 */
export function createLocalAgentServer(options: CreateLocalAgentServerOptions): LocalAgentServerHandle {
  const bridgeEnabled = options.commandBridge !== false
  let socketPath: string | null = null
  if (bridgeEnabled) {
    socketPath = options.commandBridgeSocketPath ?? join(options.host.defaultCwd, '.demi', 'command-bridge.sock')
    mkdirSync(join(options.host.defaultCwd, '.demi'), { recursive: true })
  }

  const initialEnv: Record<string, string> = {
    PATH: process.env.PATH ?? '',
    ...options.shell?.initialEnv,
  }

  const serverOptions: AgentServerOptions = {
    agent: options.agent,
    providers: options.providers,
    session: options.session,
    shell: {
      ...options.shell,
      initialEnv,
    },
  }

  if (bridgeEnabled && socketPath) {
    serverOptions.commandBridge = {
      socketPath,
      shimSource: COMMAND_BRIDGE_SHIM_SOURCE,
    }
  }

  const server = new AgentServer(serverOptions)
  let bridge: CommandBridgeHandle | null = null
  if (bridgeEnabled && socketPath) {
    bridge = startCommandBridge(server, { socketPath })
  }

  return {
    server,
    host: options.host,
    async close() {
      if (bridge) {
        await bridge.close()
        bridge = null
      }
      await server.close()
    },
  }
}
