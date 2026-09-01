import type { BashEnvironmentOptions, Host } from '@demicodes/shell'
import type { SessionPhase } from '@demicodes/core'
import type { Provider } from '@demicodes/provider'
import { AgentClient } from '../client/client'
import { createInProcessTransportPair, type AgentServerTransport } from '../protocol/transport'
import type { AgentHarness, AgentSessionStore } from '../types'
import type { TurnRetryPolicy } from '../session/retry-policy'
import type { BlobStore } from '../store/media'
import { AgentTransportBindingImpl } from './binding'
import { SessionOwnershipRegistry } from './ownership'
import {
  RunCommandLineShellNotFoundError,
  type RunCommandLineOptions,
  type RunCommandLineResult,
} from './live-session'

export {
  RunCommandLineCommandNotRegisteredError,
  RunCommandLineShellNotFoundError,
  RunCommandLineTimeoutError,
  type RunCommandLineOptions,
  type RunCommandLineResult,
} from './live-session'

/** Session tuning forwarded to every AgentSession this server creates. */
export interface AgentServerSessionOptions {
  retry?: Partial<TurnRetryPolicy>
  compaction?: {
    keepRecentTokens?: number
    preflightThresholdRatio?: number
    preflightThresholdTokens?: number
  }
  persistIntervalMs?: number
}

/**
 * Host-agnostic hook invoked before each resolved Host's BashEnvironment is built.
 * LocalHost uses this to inject PATH / env for the command bridge; AgentServer
 * itself does not know about UDS, shims, or bin directories.
 */
export interface PrepareShellContext {
  agentSessionId: string
  host: Host
  commandNames: readonly string[]
  /** Shell options from AgentServer construction (before this hook). */
  shell: Omit<BashEnvironmentOptions, 'host' | 'commands'>
}

export type PrepareShell = (
  ctx: PrepareShellContext,
) =>
  | Omit<BashEnvironmentOptions, 'host' | 'commands'>
  | Promise<Omit<BashEnvironmentOptions, 'host' | 'commands'>>

/**
 * Dynamic provider lookup: called once per runtime construction (session open,
 * `set_provider`) with the session id, so products can assemble providers per
 * connection/user and observe usage in context. Return null for unknown ids.
 */
export type ProviderResolver = (
  providerId: string,
  context: { agentSessionId: string },
) => Provider | null | Promise<Provider | null>

export interface AgentServerOptions {
  agent: AgentHarness<unknown>
  /** A static provider list, or a resolver for products that assemble providers dynamically. */
  providers: Provider[] | ProviderResolver
  shell?: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  session?: AgentServerSessionOptions
  subagents?: {
    /**
     * When false, a child closing never wakes an idle parent with an automatic
     * user send; the host app observes the `subagent closed` frame and drives
     * the parent itself. Defaults to true.
     */
    notifyParentOnIdle?: boolean
  }
  /**
   * Optional host-side shell prep (env, PATH, etc.). Implementation-agnostic —
   * command bridge wiring lives in `@demicodes/host-local`, not here.
   */
  prepareShell?: PrepareShell
  /**
   * Per-session persistence override. When absent, sessions persist through
   * the resolved Host's store (`hostAgentSessionStore` under
   * `agent-sessions/<id>`). Products with their own databases inject here.
   */
  sessionStore?: (agentSessionId: string, host: Host) => AgentSessionStore<unknown>
  /** Media blob store used by the default host-backed persistence. */
  blobs?: BlobStore
}

export interface AgentTransportBinding {
  close(): Promise<void>
}

export class AgentServer {
  private readonly agent: AgentHarness<unknown>
  private readonly resolveProvider: ProviderResolver
  private readonly shellOptions: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  private readonly sessionOptions: AgentServerSessionOptions
  private readonly prepareShell: PrepareShell | null
  private readonly notifyParentOnIdle: boolean
  private readonly sessionStore: ((agentSessionId: string, host: Host) => AgentSessionStore<unknown>) | null
  private readonly blobs: BlobStore | null
  private readonly bindings = new Set<AgentTransportBindingImpl>()
  private readonly sessionOwnership = new SessionOwnershipRegistry()

  constructor(options: AgentServerOptions) {
    this.agent = options.agent
    this.resolveProvider = Array.isArray(options.providers)
      ? staticResolver(createProviderMap(options.providers))
      : options.providers
    this.shellOptions = options.shell ?? {}
    this.sessionOptions = options.session ?? {}
    this.prepareShell = options.prepareShell ?? null
    this.notifyParentOnIdle = options.subagents?.notifyParentOnIdle ?? true
    this.sessionStore = options.sessionStore ?? null
    this.blobs = options.blobs ?? null
  }

  client(): AgentClient {
    const transports = createInProcessTransportPair()
    this.attachTransport(transports.server)
    return new AgentClient(transports.client)
  }

  attachTransport(transport: AgentServerTransport): AgentTransportBinding {
    const binding = new AgentTransportBindingImpl({
      transport,
      agent: this.agent,
      resolveProvider: this.resolveProvider,
      shell: this.shellOptions,
      session: this.sessionOptions,
      prepareShell: this.prepareShell,
      notifyParentOnIdle: this.notifyParentOnIdle,
      sessions: this.sessionOwnership,
      sessionStore: this.sessionStore,
      blobs: this.blobs,
    })
    this.bindings.add(binding)
    return binding
  }

  async close(): Promise<void> {
    const bindings = [...this.bindings]
    this.bindings.clear()
    await Promise.all(bindings.map((binding) => binding.close()))
    await this.sessionOwnership.disposeAll()
  }

  /** Current phase of a live session; null when no live session exists for the id (⇒ nothing is running). */
  sessionPhase(agentSessionId: string): SessionPhase | null {
    return this.sessionOwnership.get(agentSessionId)?.session.phase() ?? null
  }

  /**
   * Runs one registered-command invocation to completion for a live session.
   * Transport-agnostic: callers (e.g. LocalHost command bridge) supply their
   * own IPC; AgentServer only knows how to exec against the open session shell.
   */
  async runCommandLine(
    shellId: string,
    name: string,
    args: string[],
    opts: RunCommandLineOptions,
  ): Promise<RunCommandLineResult> {
    const owners = this.sessionOwnership.sessions().filter((live) => live.hasShell(shellId))
    if (owners.length === 0) throw new RunCommandLineShellNotFoundError(shellId)
    if (owners.length > 1) throw new Error(`runCommandLine: shell id "${shellId}" is not unique`)
    return owners[0]!.runCommandLine(shellId, name, args, opts)
  }
}

function createProviderMap(providers: Provider[]): Map<string, Provider> {
  const map = new Map<string, Provider>()
  for (const provider of providers) {
    if (map.has(provider.id)) throw new Error(`AgentServer: provider "${provider.id}" is already configured`)
    map.set(provider.id, provider)
  }
  return map
}

function staticResolver(map: Map<string, Provider>): ProviderResolver {
  return (providerId) => map.get(providerId) ?? null
}
