import type { CommandRegistry, Host, ShellEnvironment, ShellEnvironmentOptions } from '@demicodes/shell'
import type { SessionPhase } from '@demicodes/core'
import type { Provider } from '@demicodes/provider'
import { AgentClient } from '../client/client'
import { createInProcessTransportPair, type AgentServerTransport } from '../protocol/transport'
import type { AgentHarness, AgentTreeStore } from '../types'
import type { TurnRetryPolicy } from '../session/retry-policy'
import type { ShellPreviewBudget } from '../tools'
import type { NodeDeps } from '../node/assemble'
import { MAX_LIVE_SUBAGENTS } from '../subagent/supervisor'
import { AgentTransportBindingImpl } from './binding'
import { SessionOwnershipRegistry } from './ownership'

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

export interface ShellEnvironmentContext {
  agentSessionId: string
  host: Host
  commands: CommandRegistry
  shell: ShellEnvironmentOptions
}

/**
 * Builds the shell environment behind the `shell_*` tools for one Host. The
 * product supplies the engine per Host — the agent never knows which one
 * runs (the backend: `HostlessEnvironment` for a `VirtualHost`,
 * `RemoteShellEnvironment` for a `RemoteHost`).
 */
export type ShellEnvironmentFactory = (ctx: ShellEnvironmentContext) => ShellEnvironment | Promise<ShellEnvironment>

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
  shell?: ShellEnvironmentOptions
  session?: AgentServerSessionOptions
  subagents?: {
    /**
     * When false, a child of the root session closing never wakes the idle
     * root with an automatic user send; the host app drives the root instead.
     * Deeper levels always self-notify. Defaults to true.
     */
    notifyParentOnIdle?: boolean
    /** Maximum live direct children per session; defaults to MAX_LIVE_SUBAGENTS. */
    maxLiveSubagents?: number
  }
  tools?: {
    /** Shell preview token budget as a function of the current model's context window. */
    shellPreviewBudgetTokens?: ShellPreviewBudget
  }
  /** The shell engine per Host. */
  shellEnvironment: ShellEnvironmentFactory
  /**
   * The session tree's persistence, one store per root session id: the
   * product's own database, never a Host's store (`docs/subagent.md` §
   * Persistence). Tests and fixtures use `MemoryAgentStore` from
   * `@demicodes/agent/testing`.
   */
  store: (rootSessionId: string) => AgentTreeStore<unknown>
}

export interface AgentTransportBinding {
  close(): Promise<void>
}

export class AgentServer {
  private readonly agent: AgentHarness<unknown>
  private readonly resolveProvider: ProviderResolver
  private readonly deps: NodeDeps<unknown>
  private readonly store: (rootSessionId: string) => AgentTreeStore<unknown>
  private readonly bindings = new Set<AgentTransportBindingImpl>()
  private readonly sessionOwnership = new SessionOwnershipRegistry()

  constructor(options: AgentServerOptions) {
    this.agent = options.agent
    this.resolveProvider = Array.isArray(options.providers)
      ? staticResolver(createProviderMap(options.providers))
      : options.providers
    this.deps = {
      agent: options.agent,
      shellOptions: options.shell ?? {},
      shellEnvironment: options.shellEnvironment,
      sessionOptions: options.session ?? {},
      notifyParentOnIdle: options.subagents?.notifyParentOnIdle ?? true,
      maxLiveSubagents: options.subagents?.maxLiveSubagents ?? MAX_LIVE_SUBAGENTS,
      shellPreviewBudgetTokens: options.tools?.shellPreviewBudgetTokens ?? null,
    }
    this.store = options.store
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
      deps: this.deps,
      store: this.store,
      sessions: this.sessionOwnership,
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
