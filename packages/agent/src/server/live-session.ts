import { bytesToBase64, errorCode, noop } from '@demicodes/utils'
import { type ShellEnvironment, type CommandRegistry, type Host, type ShellEnvironmentOptions } from '@demicodes/shell'
import type { AgentSession } from '../session/session'
import type { ChildSupervisor } from '../subagent/supervisor'
import type { ServerFrame } from '../protocol/frames'
import type { AgentHarness, AgentToolInvokeContext, SessionEvent } from '../types'
import type { ShellEnvironmentFactory } from './server'
import { errorDiagnostics, progressToOutput, progressToShellOutput } from './summaries'

export interface LiveSessionOptions {
  agentSessionId: string
  session: AgentSession<unknown>
  supervisor: ChildSupervisor<unknown>
  agent: AgentHarness<unknown>
  commandRegistry: CommandRegistry
  cwd: string
  providerId: string
  shellOptions: ShellEnvironmentOptions
  shellEnvironment: ShellEnvironmentFactory
}

/**
 * One running session, owned by the server (via the ownership registry), not
 * by any transport binding. It carries everything session-scoped — the
 * AgentSession, subagent supervisor, per-Host shell environments — and emits
 * server frames through a swappable sink: the currently attached binding, or
 * nowhere while detached (turns keep running either way).
 */
export class LiveSession {
  readonly agentSessionId: string
  readonly session: AgentSession<unknown>
  readonly supervisor: ChildSupervisor<unknown>
  readonly agent: AgentHarness<unknown>
  readonly commandRegistry: CommandRegistry
  readonly commandNames: ReadonlySet<string>
  readonly cwd: string
  providerId: string
  sink: (frame: ServerFrame) => void = noop

  private readonly shellOptions: ShellEnvironmentOptions
  private readonly shellEnvironment: ShellEnvironmentFactory
  private readonly environmentsByHost = new Map<Host, ShellEnvironment>()
  private readonly pendingEnvironmentsByHost = new Map<Host, Promise<ShellEnvironment>>()
  private readonly unsubscribeSession: () => void

  constructor(options: LiveSessionOptions) {
    this.agentSessionId = options.agentSessionId
    this.session = options.session
    this.supervisor = options.supervisor
    this.agent = options.agent
    this.commandRegistry = options.commandRegistry
    this.commandNames = new Set(options.commandRegistry.list().map((command) => command.name))
    this.cwd = options.cwd
    this.providerId = options.providerId
    this.shellOptions = options.shellOptions
    this.shellEnvironment = options.shellEnvironment
    this.unsubscribeSession = options.session.subscribe((event) => this.handleSessionEvent(event))
  }

  attachSink(sink: (frame: ServerFrame) => void): void {
    this.sink = sink
  }

  detachSink(): void {
    this.sink = noop
  }

  async dispose(): Promise<void> {
    try {
      await this.supervisor.dispose()
      await this.session.dispose()
      const pendingEnvironments = await Promise.allSettled(this.pendingEnvironmentsByHost.values())
      const environments = new Set(this.environmentsByHost.values())
      for (const result of pendingEnvironments) {
        if (result.status === 'fulfilled') environments.add(result.value)
      }
      await Promise.all([...environments].map((environment) => environment.disposeAllShells()))
      await this.agent.dispose?.({
        agentSessionId: this.session.id(),
        state: this.session.state(),
        cwd: this.cwd,
        transcript: this.session.transcript(),
      })
    } finally {
      this.unsubscribeSession()
      this.detachSink()
      this.environmentsByHost.clear()
      this.pendingEnvironmentsByHost.clear()
    }
  }

  private handleSessionEvent(event: SessionEvent): void {
    switch (event.type) {
      case 'transcript_changed':
        this.sink({ type: 'transcript_patch', patches: event.patches, revision: event.revision })
        return
      case 'phase_changed':
        this.sink({ type: 'phase', phase: event.phase })
        return
      case 'queue_changed':
        this.sink({ type: 'queue', queue: event.queue })
        return
      case 'tool_progress': {
        this.emitToolProgress(event.toolCallId, event.toolName, event.progress)
        return
      }
      case 'retry_scheduled':
        this.sink({
          type: 'retry_scheduled',
          attempt: event.attempt,
          delayMs: event.delayMs,
          code: event.code,
          diagnostics: event.diagnostics,
        })
        return
      case 'error': {
        const normalized = event.error instanceof Error ? event.error : new Error(String(event.error))
        const code = errorCode(event.error)
        const diagnostics = errorDiagnostics(event.error)
        this.sink({
          type: 'error',
          message: normalized.message,
          ...(code ? { code } : {}),
          ...(diagnostics ? { diagnostics } : {}),
        })
        return
      }
    }
  }

  private emitToolProgress(toolCallId: string, toolName: string, progress: unknown): void {
    const output = progressToOutput(progress)
    this.sink({ type: 'tool_progress', toolUseId: toolCallId, output })
    const shell = toolName === 'shell_status' ? null : progressToShellOutput(progress)
    if (shell) {
      this.sink({
        type: 'shell_output',
        shellId: shell.shellId,
        commandId: shell.commandId,
        status: shell.status,
      })
    }
  }

  hasShell(shellId: string): boolean {
    return this.environmentForShell(shellId) !== null || this.supervisor.hasShell(shellId)
  }

  async resolveEnvironment(
    ctx: Pick<AgentToolInvokeContext<unknown>, 'agentSessionId' | 'state' | 'cwd' | 'metadata'>,
    handle: { shellId?: string; commandId?: string },
  ): Promise<ShellEnvironment> {
    const host = await this.agent.host({
      agentSessionId: ctx.agentSessionId,
      state: ctx.state,
      cwd: ctx.cwd,
      metadata: ctx.metadata,
    })
    const environment = await this.environmentForHost(host, this.commandRegistry)
    const owner = handle.shellId
      ? this.environmentForShell(handle.shellId)
      : handle.commandId
        ? this.environmentForCommand(handle.commandId)
        : null
    if (owner && owner !== environment) {
      const id = handle.shellId ?? handle.commandId
      throw new Error(`Shell handle "${id}" belongs to a different Host`)
    }
    return environment
  }

  private async environmentForHost(host: Host, commands: CommandRegistry): Promise<ShellEnvironment> {
    const existing = this.environmentsByHost.get(host)
    if (existing) return existing
    const pending = this.pendingEnvironmentsByHost.get(host)
    if (pending) return pending
    const creation = this.createEnvironment(host, commands)
    this.pendingEnvironmentsByHost.set(host, creation)
    try {
      const environment = await creation
      this.environmentsByHost.set(host, environment)
      return environment
    } finally {
      this.pendingEnvironmentsByHost.delete(host)
    }
  }

  private async createEnvironment(host: Host, commands: CommandRegistry): Promise<ShellEnvironment> {
    const shellOptions = this.shellOptions
    return this.shellEnvironment({ agentSessionId: this.agentSessionId, host, commands, shell: shellOptions })
  }

  /** The distinct environments of this session: a product may serve two Hosts with one object (a target that changed engines underneath). */
  private environments(): ShellEnvironment[] {
    return [...new Set(this.environmentsByHost.values())]
  }

  private environmentForShell(shellId: string): ShellEnvironment | null {
    const matches = this.environments().filter((environment) => environment.getShell(shellId))
    if (matches.length > 1) throw new Error(`Shell id "${shellId}" is not unique in this session`)
    return matches[0] ?? null
  }

  private environmentForCommand(commandId: string): ShellEnvironment | null {
    const matches = this.environments().filter((environment) => environment.hasCommand(commandId))
    if (matches.length > 1) throw new Error(`Command id "${commandId}" is not unique in this session`)
    return matches[0] ?? null
  }
}
