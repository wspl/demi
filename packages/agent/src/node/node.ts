import type { QueuedMessage } from '@demicodes/core'
import type { CommandRegistry, Host, ShellEnvironment, ShellEnvironmentOptions } from '@demicodes/shell'
import type { ShellEnvironmentFactory } from '../server/server'
import type { AgentSession } from '../session/session'
import type { ChildSupervisor } from '../subagent/supervisor'
import type { AgentHarness, AgentNodeRecord, AgentToolInvokeContext } from '../types'

/**
 * How a node behaves at its lifecycle edges — the one thing that differs
 * between the root and a subagent (`docs/subagent.md` § Runtime).
 */
export interface NodePolicy {
  /** On restore, resume a turn the process interrupted; the root leaves that to its client. */
  resumeInterrupted: boolean
  /** Close when quiescent — idle, empty inbox, no live children — and report to the parent. */
  closeWhenDone: boolean
}

export const ROOT_POLICY: NodePolicy = { resumeInterrupted: false, closeWhenDone: false }
export const CHILD_POLICY: NodePolicy = { resumeInterrupted: true, closeWhenDone: true }

/** What a node has yet to run: the turn the process interrupted, the messages queued in its checkpoint. */
export interface NodeContinuation {
  interrupted: boolean
  queued: QueuedMessage[]
}

export interface SessionNodeOptions<State> {
  record: AgentNodeRecord
  session: AgentSession<State>
  supervisor: ChildSupervisor<State>
  agent: AgentHarness<State>
  commandRegistry: CommandRegistry
  cwd: string
  /** The root's id: every node resolves its Host as the root, the execution target being the conversation's. */
  hostSessionId: string
  shellOptions: ShellEnvironmentOptions
  shellEnvironment: ShellEnvironmentFactory
  policy: NodePolicy
  continuation: NodeContinuation | null
}

/**
 * One node of the session tree at run time: its session, the supervisor of
 * its children, its command tree, and its shell environments — one per Host
 * it has acted on, with handle ownership checked across them. The same
 * class for the root and every subagent.
 */
export class SessionNode<State = unknown> {
  readonly id: string
  readonly record: AgentNodeRecord
  readonly session: AgentSession<State>
  readonly supervisor: ChildSupervisor<State>
  readonly agent: AgentHarness<State>
  readonly commandRegistry: CommandRegistry
  readonly commandNames: ReadonlySet<string>
  readonly cwd: string
  readonly policy: NodePolicy

  private readonly hostSessionId: string
  private readonly shellOptions: ShellEnvironmentOptions
  private readonly shellEnvironment: ShellEnvironmentFactory
  private continuation: NodeContinuation | null
  private readonly environmentsByHost = new Map<Host, ShellEnvironment>()
  private readonly pendingEnvironmentsByHost = new Map<Host, Promise<ShellEnvironment>>()

  constructor(options: SessionNodeOptions<State>) {
    this.id = options.record.id
    this.record = options.record
    this.session = options.session
    this.supervisor = options.supervisor
    this.agent = options.agent
    this.commandRegistry = options.commandRegistry
    this.commandNames = new Set(options.commandRegistry.list().map((command) => command.name))
    this.cwd = options.cwd
    this.policy = options.policy
    this.hostSessionId = options.hostSessionId
    this.shellOptions = options.shellOptions
    this.shellEnvironment = options.shellEnvironment
    this.continuation = options.continuation
  }

  /**
   * Runs what the node has yet to run, under its policy: the interrupted
   * turn when the policy resumes it, then the queued messages in order —
   * a fresh child's brief among them. Every turn goes to `track`; the
   * owner decides what a failure means.
   */
  continue(track: (turn: Promise<void>) => void): void {
    const continuation = this.continuation
    this.continuation = null
    if (!continuation) return
    const metadata = this.record.metadata ?? undefined
    const options = metadata ? { metadata } : {}
    if (continuation.interrupted && this.policy.resumeInterrupted) track(this.session.resume(options))
    for (const message of continuation.queued) track(this.session.send(message.content, { id: message.id, ...options }))
  }

  hasShell(shellId: string): boolean {
    return this.environmentForShell(shellId) !== null || this.supervisor.hasShell(shellId)
  }

  /** The distinct environments of this node: a product may serve two Hosts with one object. */
  environments(): ShellEnvironment[] {
    return [...new Set(this.environmentsByHost.values())]
  }

  /**
   * The shell environment behind a tool call: the Host the action's metadata
   * names, resolved as the root; a handle the call carries must belong to
   * that Host's environment.
   */
  async resolveEnvironment(
    ctx: Pick<AgentToolInvokeContext<State>, 'state' | 'metadata'>,
    handle: { shellId?: string; commandId?: string },
  ): Promise<ShellEnvironment> {
    const host = await this.agent.host({ agentSessionId: this.hostSessionId, state: ctx.state, cwd: this.cwd, metadata: ctx.metadata })
    const environment = await this.environmentForHost(host)
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

  /** Every shell this node opened, on every Host, including those still being created. */
  async disposeEnvironments(): Promise<void> {
    const pending = await Promise.allSettled(this.pendingEnvironmentsByHost.values())
    const environments = new Set(this.environmentsByHost.values())
    for (const result of pending) {
      if (result.status === 'fulfilled') environments.add(result.value)
    }
    this.environmentsByHost.clear()
    this.pendingEnvironmentsByHost.clear()
    await Promise.all([...environments].map((environment) => environment.disposeAllShells().catch(() => {})))
  }

  private async environmentForHost(host: Host): Promise<ShellEnvironment> {
    const existing = this.environmentsByHost.get(host)
    if (existing) return existing
    const pending = this.pendingEnvironmentsByHost.get(host)
    if (pending) return pending
    const creation = Promise.resolve(
      this.shellEnvironment({ agentSessionId: this.id, host, commands: this.commandRegistry, shell: this.shellOptions }),
    )
    this.pendingEnvironmentsByHost.set(host, creation)
    try {
      const environment = await creation
      this.environmentsByHost.set(host, environment)
      return environment
    } finally {
      this.pendingEnvironmentsByHost.delete(host)
    }
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
