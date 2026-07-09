import { mkdirSync } from 'node:fs'
import { dirname } from 'node:path'
import {
  AgentServer,
  type AgentHarness,
  type AgentServerOptions,
  type AgentServerSessionOptions,
} from '@demicodes/agent'
import type { Provider } from '@demicodes/provider'
import type { BashEnvironmentOptions } from '@demicodes/shell'
import { COMMAND_BRIDGE_SHIM_SOURCE, startCommandBridge, type CommandBridgeHandle } from './command-bridge'
import { materializeCommandBridgeShims } from './command-bridge-shim'
import { defaultBridgeSocketPath, resolveDemiHome } from './demi-home'
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
   * Demi state root (default `$DEMI_HOME` or `~/.demi`).
   * Bridge socket and `bridge-bin/` always live under this tree — never under workspace cwd.
   * Layout is fixed: `bridges/*.sock`, `bridge-bin/<sessionId>/`.
   */
  stateDir?: string
  /** Override UDS path when bridge is enabled (default under `stateDir/bridges/`). */
  commandBridgeSocketPath?: string
}

export interface LocalAgentServerHandle {
  server: AgentServer
  host: LocalHost
  /** Resolved state root used for bridge artifacts when bridge is enabled. */
  stateDir: string | null
  /** Stops the bridge listener (if any) then closes the server. */
  close(): Promise<void>
}

/**
 * Open-box local agent assembly: `LocalHost` + `AgentServer` with command
 * bridge **on by default**.
 *
 * Owns the Node-only bridge transport (UDS + PATH shims under `stateDir`).
 * AgentServer only receives a generic `prepareSessionShell` hook and exposes
 * `runCommandLine` — it never sees bin dirs or sockets.
 *
 * Default layout under `stateDir` (`~/.demi` or `$DEMI_HOME`):
 * ```
 * bridges/<id>.sock
 * bridge-bin/<sessionId>/…
 * ```
 */
export function createLocalAgentServer(options: CreateLocalAgentServerOptions): LocalAgentServerHandle {
  const bridgeEnabled = options.commandBridge !== false
  const stateDir = bridgeEnabled ? resolveDemiHome(options.stateDir) : null

  let socketPath: string | null = null
  if (bridgeEnabled && stateDir) {
    socketPath = options.commandBridgeSocketPath ?? defaultBridgeSocketPath(stateDir)
    mkdirSync(dirname(socketPath), { recursive: true })
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

  if (bridgeEnabled && socketPath && stateDir) {
    const bridgeStateDir = stateDir
    const bridgeSocketPath = socketPath
    serverOptions.prepareSessionShell = async ({ host, agentSessionId, commandNames, shell }) => {
      const shimDir = await materializeCommandBridgeShims({
        host,
        agentSessionId,
        commandNames,
        shimSource: COMMAND_BRIDGE_SHIM_SOURCE,
        stateDir: bridgeStateDir,
      })
      const existingPath = shell.initialEnv?.PATH
      return {
        ...shell,
        initialEnv: {
          ...shell.initialEnv,
          DEMI_COMMAND_BRIDGE_SOCK: bridgeSocketPath,
          PATH: existingPath ? `${shimDir}:${existingPath}` : shimDir,
        },
      }
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
    stateDir,
    async close() {
      if (bridge) {
        await bridge.close()
        bridge = null
      }
      await server.close()
    },
  }
}
