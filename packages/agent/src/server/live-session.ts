import { bytesToBase64, errorCode, noop } from '@demicodes/utils'
import {
  type ShellEnvironment,
  MAX_TIMEOUT_MS,
  heredocDelimiter,
  shellQuote,
  type CommandRegistry,
  type Host,
} from '@demicodes/shell'
import type { BashEnvironmentOptions } from '@demicodes/shell/bash'
import type { AgentSession } from '../session/session'
import type { ChildSupervisor } from '../subagent/supervisor'
import type { ServerFrame } from '../protocol/frames'
import type { AgentHarness, AgentToolInvokeContext, SessionEvent } from '../types'
import type { PrepareShell, ShellEnvironmentFactory } from './server'
import { errorDiagnostics, progressToAudit, progressToOutput, progressToShellOutput } from './summaries'

/** Options for {@link AgentServer.runCommandLine}. */
export interface RunCommandLineOptions {
  cwd: string
  stdin: string
  signal?: AbortSignal
}

/** Result of {@link AgentServer.runCommandLine}. */
export interface RunCommandLineResult {
  exitCode: number
  /** UTF-8 text, or base64 when `stdoutEncoding` is 'base64'. */
  stdout: string
  stdoutEncoding?: 'base64'
  stderr: string
}

export class RunCommandLineShellNotFoundError extends Error {
  constructor(readonly shellId: string) {
    super(`runCommandLine: shell "${shellId}" is not open in this process`)
    this.name = 'RunCommandLineShellNotFoundError'
  }
}

export class RunCommandLineCommandNotRegisteredError extends Error {
  constructor(readonly commandName: string) {
    super(`runCommandLine: command "${commandName}" is not registered for this session`)
    this.name = 'RunCommandLineCommandNotRegisteredError'
  }
}

export class RunCommandLineTimeoutError extends Error {
  constructor(
    readonly commandId: string,
    readonly partialStdout: string,
    readonly partialStderr: string,
  ) {
    super(`runCommandLine: command "${commandId}" exceeded the ${MAX_TIMEOUT_MS}ms ceiling and was aborted`)
    this.name = 'RunCommandLineTimeoutError'
  }
}

export interface LiveSessionOptions {
  agentSessionId: string
  session: AgentSession<unknown>
  supervisor: ChildSupervisor<unknown>
  agent: AgentHarness<unknown>
  commandRegistry: CommandRegistry
  cwd: string
  providerId: string
  shellOptions: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  prepareShell: PrepareShell | null
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

  private readonly shellOptions: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  private readonly prepareShell: PrepareShell | null
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
    this.prepareShell = options.prepareShell
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
    const audit = progressToAudit(progress)
    if (audit.length > 0) this.sink({ type: 'audit', events: audit })
  }

  /** Backs `AgentServer.runCommandLine` — see its doc comment for the contract. */
  async runCommandLine(
    shellId: string,
    name: string,
    args: string[],
    opts: RunCommandLineOptions,
  ): Promise<RunCommandLineResult> {
    // Scope resolution doubles as the subagent boundary: a child shell resolves
    // to its own environment and command set, which has no spawn surface.
    const parentEnvironment = this.environmentForShell(shellId)
    const scope = parentEnvironment
      ? {
          environment: parentEnvironment,
          commandNames: this.commandNames,
          agentSessionId: this.agentSessionId,
        }
      : (this.supervisor.environmentScopeForShell(shellId) ?? null)
    if (!scope) throw new Error('runCommandLine: session has no active shell environment')
    const { environment, agentSessionId } = scope
    if (!scope.commandNames.has(name)) throw new RunCommandLineCommandNotRegisteredError(name)

    const words = [name, ...args].map(shellQuote).join(' ')
    let script = words
    if (opts.stdin.length > 0) {
      const delimiter = heredocDelimiter(opts.stdin)
      // A heredoc body always ends with a newline; add one only when the piped
      // stdin lacks it, so newline-terminated input arrives byte-identical.
      const body = opts.stdin.endsWith('\n') ? opts.stdin : `${opts.stdin}\n`
      script = `${words} <<'${delimiter}'\n${body}${delimiter}`
    }

    // Ephemeral shell born in the caller's cwd: the bridge caller's directory
    // and env must never leak into the model's persistent session shell.
    const result = await environment.exec({
      agentSessionId,
      script,
      timeoutMs: MAX_TIMEOUT_MS,
      signal: opts.signal,
      ephemeral: true,
      cwd: opts.cwd,
    })

    try {
      if (result.status === 'exited') {
        // Binary final streams cross the bridge as base64; the shim writes the
        // raw bytes to its OS stdout, keeping external pipes byte-clean.
        if (result.binaryStdout) {
          const truncationNote = result.binaryStdout.truncated
            ? `command bridge: binary stdout truncated at the output limit (${result.binaryStdout.data.length} of ${result.binaryStdout.totalBytes} bytes)\n`
            : ''
          return {
            exitCode: result.exitCode,
            stdout: bytesToBase64(result.binaryStdout.data),
            stdoutEncoding: 'base64',
            stderr: `${result.stderr.delta}${truncationNote}`,
          }
        }
        return { exitCode: result.exitCode, stdout: result.stdout.delta, stderr: result.stderr.delta }
      }
      if (result.status === 'aborted') {
        throw new Error(`runCommandLine: call for "${name}" was cancelled before it completed`)
      }
      const aborted = await environment.abort({ commandId: result.commandId })
      throw new RunCommandLineTimeoutError(
        result.commandId,
        aborted.status === 'aborted' ? aborted.stdout.delta : '',
        aborted.status === 'aborted' ? aborted.stderr.delta : '',
      )
    } finally {
      await environment.disposeShell(result.shellId).catch(() => {})
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
    const shellOptions = this.prepareShell
      ? await this.prepareShell({
          agentSessionId: this.agentSessionId,
          host,
          commandNames: commands.list().map((command) => command.name),
          shell: this.shellOptions,
        })
      : this.shellOptions
    return this.shellEnvironment({ agentSessionId: this.agentSessionId, host, commands, shell: shellOptions })
  }

  private environmentForShell(shellId: string): ShellEnvironment | null {
    const matches = [...this.environmentsByHost.values()].filter((environment) => environment.getShell(shellId))
    if (matches.length > 1) throw new Error(`Shell id "${shellId}" is not unique in this session`)
    return matches[0] ?? null
  }

  private environmentForCommand(commandId: string): ShellEnvironment | null {
    const matches = [...this.environmentsByHost.values()].filter((environment) => environment.hasCommand(commandId))
    if (matches.length > 1) throw new Error(`Command id "${commandId}" is not unique in this session`)
    return matches[0] ?? null
  }
}
