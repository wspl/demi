import type { CommandRegistry, Host, ShellEnvironment, ShellEnvironmentOptions } from '@demicodes/shell'
import type { SessionPhase } from '@demicodes/core'
import type { Provider } from '@demicodes/provider'
import { AgentClient } from '../client/client'
import { createInProcessTransportPair, type AgentServerTransport } from '../protocol/transport'
import type { AgentHarness, AgentSessionStore } from '../types'
import type { TurnRetryPolicy } from '../session/retry-policy'
import type { BlobStore } from '../store/media'
import type { ShellPreviewBudget } from '../tools'
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
   * Per-session persistence override. When absent, sessions persist through
   * the resolved Host's store (`hostAgentSessionStore` under
   * `agent-sessions/<id>`). Products with their own databases inject here.
   */
  sessionStore?: (agentSessionId: string, host: Host) => AgentSessionStore<unknown>
  /** Media store for a root session and its children, scoped by the composing product. */
  blobs?: (agentSessionId: string) => BlobStore
}

export interface AgentTransportBinding {
  close(): Promise<void>
}

export class AgentServer {
  private readonly agent: AgentHarness<unknown>
  private readonly resolveProvider: ProviderResolver
  private readonly shellOptions: ShellEnvironmentOptions
  private readonly sessionOptions: AgentServerSessionOptions
  private readonly shellEnvironment: ShellEnvironmentFactory
  private readonly notifyParentOnIdle: boolean
  private readonly maxLiveSubagents: number
  private readonly shellPreviewBudgetTokens: ShellPreviewBudget | null
  private readonly sessionStore: ((agentSessionId: string, host: Host) => AgentSessionStore<unknown>) | null
  private readonly blobs: NonNullable<AgentServerOptions['blobs']> | null
  private readonly bindings = new Set<AgentTransportBindingImpl>()
  private readonly sessionOwnership = new SessionOwnershipRegistry()

  constructor(options: AgentServerOptions) {
    this.agent = options.agent
    this.resolveProvider = Array.isArray(options.providers)
      ? staticResolver(createProviderMap(options.providers))
      : options.providers
    this.shellOptions = options.shell ?? {}
    this.sessionOptions = options.session ?? {}
    this.shellEnvironment = options.shellEnvironment
    this.notifyParentOnIdle = options.subagents?.notifyParentOnIdle ?? true
    this.maxLiveSubagents = options.subagents?.maxLiveSubagents ?? MAX_LIVE_SUBAGENTS
    this.shellPreviewBudgetTokens = options.tools?.shellPreviewBudgetTokens ?? null
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
      shellEnvironment: this.shellEnvironment,
      notifyParentOnIdle: this.notifyParentOnIdle,
      maxLiveSubagents: this.maxLiveSubagents,
      shellPreviewBudgetTokens: this.shellPreviewBudgetTokens,
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
