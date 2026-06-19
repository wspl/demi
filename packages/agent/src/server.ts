import { AgentSession } from './session'
import {
  BashEnvironment,
  CommandRegistry,
  createShellSessionTools,
  type BashAuditEvent,
  type BashEnvironmentOptions,
} from '@demi/shell'
import type { Block, ToolResultContentBlock } from '@demi/core'
import type { ProviderRegistry } from '@demi/provider'
import { AgentClient } from './client'
import { cloneBlocks, diffTranscriptBlocks } from './patch'
import type { ClientFrame, OutputSnapshotLike, ServerFrame } from './frames'
import { createInProcessTransportPair, type AgentServerTransport } from './transport'
import type { AgentHarness, AgentHarnessRuntime, SessionEvent } from './types'

export interface AgentServerOptions {
  agent: AgentHarness<unknown>
  providerRegistry: ProviderRegistry
  shell?: Omit<BashEnvironmentOptions, 'host' | 'commands'>
}

export interface AgentTransportBinding {
  close(): Promise<void>
}

export class AgentServer {
  private readonly agent: AgentHarness<unknown>
  private readonly providerRegistry: ProviderRegistry
  private readonly shellOptions: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  private readonly bindings = new Set<AgentTransportBindingImpl>()

  constructor(options: AgentServerOptions) {
    this.agent = options.agent
    this.providerRegistry = options.providerRegistry
    this.shellOptions = options.shell ?? {}
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
      providerRegistry: this.providerRegistry,
      shell: this.shellOptions,
    })
    this.bindings.add(binding)
    return binding
  }

  async close(): Promise<void> {
    const bindings = [...this.bindings]
    this.bindings.clear()
    await Promise.all(bindings.map((binding) => binding.close()))
  }
}

interface AgentTransportBindingOptions {
  transport: AgentServerTransport
  agent: AgentHarness<unknown>
  providerRegistry: ProviderRegistry
  shell?: Omit<BashEnvironmentOptions, 'host' | 'commands'>
}

class AgentTransportBindingImpl implements AgentTransportBinding {
  private readonly transport: AgentServerTransport
  private readonly agent: AgentHarness<unknown>
  private readonly providerRegistry: ProviderRegistry
  private readonly shellOptions: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  private session: AgentSession<unknown> | null = null
  private currentAgent: AgentHarness<unknown> | null = null
  private currentEnvironment: BashEnvironment | null = null
  private currentCwd: string | null = null
  private unsubscribeSession: (() => void) | null = null
  private lastTranscriptBlocks: Block[] = []
  private unsubscribeTransport: (() => void) | null = null
  private closed = false

  constructor(options: AgentTransportBindingOptions) {
    this.transport = options.transport
    this.agent = options.agent
    this.providerRegistry = options.providerRegistry
    this.shellOptions = options.shell ?? {}
    this.unsubscribeTransport = this.transport.onFrame((frame) => {
      void this.handleFrame(frame)
    })
  }

  async close(): Promise<void> {
    if (this.closed) return
    this.closed = true
    try {
      await this.closeSession()
    } catch (error) {
      this.sendError(error)
    } finally {
      this.unsubscribeTransport?.()
      this.unsubscribeTransport = null
      this.transport.close()
    }
  }

  private async handleFrame(frame: ClientFrame): Promise<void> {
    try {
      switch (frame.type) {
        case 'open':
          await this.open(frame)
          return
        case 'send': {
          const session = this.sessionFor('send')
          if (!session) return
          this.observeSessionAction(session.send(frame.content))
          return
        }
        case 'retry': {
          const session = this.sessionFor('retry')
          if (!session || this.rejectIfBusy(session, 'retry')) return
          this.observeSessionAction(session.retry())
          return
        }
        case 'resume': {
          const session = this.sessionFor('resume')
          if (!session || this.rejectIfBusy(session, 'resume')) return
          this.observeSessionAction(session.resume())
          return
        }
        case 'compact': {
          const session = this.sessionFor('compact')
          if (!session || this.rejectIfBusy(session, 'compact')) return
          this.observeSessionAction(session.compact())
          return
        }
        case 'abort': {
          const session = this.sessionFor('abort')
          if (!session) return
          await session.abort()
          return
        }
        case 'shell_input':
          await this.handleShellInput(frame)
          return
        case 'close':
          await this.closeSession()
          this.send({ type: 'closed' })
          return
      }
    } catch (error) {
      this.sendError(error)
    }
  }

  private async open(frame: Extract<ClientFrame, { type: 'open' }>): Promise<void> {
    if (this.session) {
      this.send({ type: 'rejected', command: 'open', reason: 'A session is already open on this connection' })
      return
    }

    const agent = this.agent
    const provider = await this.providerRegistry.createProvider(frame.provider.type, frame.provider.config)
    const state = agent.initialState()
    const harnessContext = { state, cwd: frame.cwd }
    const commands = agent.commands?.(harnessContext) ?? []
    const commandRegistry = new CommandRegistry()
    for (const command of commands) commandRegistry.register(command)
    const environment = new BashEnvironment({
      ...this.shellOptions,
      host: agent.host(harnessContext),
      commands: commandRegistry,
    })
    const tools = createShellSessionTools(environment)
    const runtime: AgentHarnessRuntime<unknown> = {
      harnessName: agent.name,
      initialState: () => state,
      systemPrompt: (ctx) => agent.systemPrompt(ctx),
      preamble: (ctx) => agent.preamble?.(ctx) ?? null,
      resolveReferences: (ctx, content) => agent.resolveReferences?.(ctx, content) ?? content,
      lifecycle: (event) => agent.lifecycle?.(event),
      tools: () => tools,
    }
    this.session = new AgentSession({
      provider,
      model: frame.provider.model,
      cwd: frame.cwd,
      runtime,
      state,
    })
    this.currentAgent = agent
    this.currentEnvironment = environment
    this.currentCwd = frame.cwd
    this.lastTranscriptBlocks = []
    this.unsubscribeSession = this.session.subscribe((event) => this.handleSessionEvent(event))

    this.send({ type: 'opened' })
    this.send({ type: 'transcript_snapshot', blocks: [] })
    this.send({ type: 'phase', phase: this.session.phase() })
    this.send({ type: 'queue', queue: this.session.queuedMessages() })
  }

  private handleSessionEvent(event: SessionEvent): void {
    switch (event.type) {
      case 'transcript_changed': {
        const next = event.transcript.blocks
        const patches = diffTranscriptBlocks(this.lastTranscriptBlocks, next)
        this.lastTranscriptBlocks = cloneBlocks(next)
        if (patches.length > 0) this.send({ type: 'transcript_patch', patches })
        return
      }
      case 'phase_changed':
        this.send({ type: 'phase', phase: event.phase })
        return
      case 'queue_changed':
        this.send({ type: 'queue', queue: event.queue })
        return
      case 'tool_progress': {
        this.sendToolProgress(event.toolCallId, event.progress)
        return
      }
      case 'error':
        this.sendError(event.error)
        return
    }
  }

  private async closeSession(): Promise<void> {
    const session = this.session
    const agent = this.currentAgent
    const environment = this.currentEnvironment
    const cwd = this.currentCwd

    try {
      if (session) await session.abort()
      if (environment) await environment.disposeAllShells()
      if (session && agent && cwd) {
        await agent.dispose?.({ agentSessionId: session.id(), state: session.state(), cwd, transcript: session.transcript() })
      }
    } finally {
      this.unsubscribeSession?.()
      this.unsubscribeSession = null
      this.session = null
      this.currentAgent = null
      this.currentEnvironment = null
      this.currentCwd = null
      this.lastTranscriptBlocks = []
    }
  }

  private async handleShellInput(frame: Extract<ClientFrame, { type: 'shell_input' }>): Promise<void> {
    const session = this.sessionFor('shell_input')
    if (!session || !this.currentEnvironment || !this.currentCwd) return

    const tools = createShellSessionTools(this.currentEnvironment)
    const tool = tools.find((candidate) => candidate.name === 'shell_input')
    if (!tool) {
      this.send({ type: 'rejected', command: 'shell_input', reason: 'Current harness does not expose shell_input' })
      return
    }

    let emittedProgress = false
    const result = await tool.invoke(
      {
        agentSessionId: session.id(),
        state: session.state(),
        cwd: this.currentCwd,
        toolCallId: frame.shellId,
        signal: new AbortController().signal,
        emitProgress: (progress) => {
          emittedProgress = true
          this.sendShellInputResult(frame.shellId, progress)
        },
      },
      { shellId: frame.shellId, stdin: frame.stdin },
    )
    if (!emittedProgress) this.sendShellInputResult(frame.shellId, result.metadata ?? result.output)
  }

  private sessionFor(command: string): AgentSession<unknown> | null {
    if (!this.session) {
      this.send({ type: 'rejected', command, reason: 'No session is open' })
      return null
    }
    return this.session
  }

  private rejectIfBusy(session: AgentSession<unknown>, command: string): boolean {
    const phase = session.phase()
    if (phase === 'idle') return false
    this.send({ type: 'rejected', command, reason: `Session is busy (${phase})` })
    return true
  }

  private sendToolProgress(toolCallId: string, progress: unknown): void {
    const output = progressToOutput(progress)
    this.send({ type: 'tool_progress', toolUseId: toolCallId, output })
    const shell = progressToShellOutput(progress)
    if (shell) this.send({ type: 'shell_output', shellId: shell.shellId, snapshot: shell.snapshot })
    const audit = progressToAudit(progress)
    if (audit.length > 0) this.send({ type: 'audit', events: audit })
  }

  private sendShellInputResult(shellId: string, progress: unknown): void {
    const shell = progressToShellOutput(progress)
    if (shell) this.send({ type: 'shell_output', shellId: shell.shellId, snapshot: shell.snapshot })
    const audit = progressToAudit(progress)
    if (audit.length > 0) this.send({ type: 'audit', events: audit })
    this.send({ type: 'shell_input_result', shellId, output: progressToOutput(progress) })
  }

  private send(frame: ServerFrame): void {
    this.transport.send(frame)
  }

  private observeSessionAction(action: Promise<void>): void {
    action.catch(noop)
  }

  private sendError(error: unknown): void {
    const normalized = error instanceof Error ? error : new Error(String(error))
    const code = errorCode(error)
    this.send(code ? { type: 'error', message: normalized.message, code } : { type: 'error', message: normalized.message })
  }
}

function progressToOutput(progress: unknown): ToolResultContentBlock[] {
  return [{ type: 'text', text: progressToText(progress) }]
}

function progressToText(progress: unknown): string {
  if (typeof progress === 'string') return progress
  if (typeof progress === 'bigint') return progress.toString()
  if (typeof progress === 'symbol') return String(progress)
  if (typeof progress === 'function') return `[Function ${progress.name || 'anonymous'}]`
  try {
    return safeStringify(progress) ?? String(progress)
  } catch {
    return String(progress)
  }
}

function safeStringify(value: unknown): string | undefined {
  const seen = new WeakSet<object>()
  return JSON.stringify(value, (_key, nested) => {
    if (typeof nested === 'bigint') return nested.toString()
    if (typeof nested === 'symbol') return String(nested)
    if (typeof nested === 'function') return `[Function ${nested.name || 'anonymous'}]`
    if (nested !== null && typeof nested === 'object') {
      if (seen.has(nested)) return '[Circular]'
      seen.add(nested)
    }
    return nested
  })
}

function progressToShellOutput(progress: unknown): { shellId: string; snapshot: OutputSnapshotLike } | null {
  if (!isRecord(progress)) return null
  if (typeof progress.shellId !== 'string' || !isRecord(progress.output)) return null
  const output = progress.output
  if (
    typeof output.stdoutDelta !== 'string' ||
    typeof output.stderrDelta !== 'string' ||
    typeof output.stdoutTail !== 'string' ||
    typeof output.stderrTail !== 'string' ||
    typeof output.totalStdoutBytes !== 'number' ||
    typeof output.totalStderrBytes !== 'number' ||
    typeof output.truncated !== 'boolean'
  ) {
    return null
  }
  return { shellId: progress.shellId, snapshot: output as unknown as OutputSnapshotLike }
}

function progressToAudit(progress: unknown): BashAuditEvent[] {
  if (!isRecord(progress) || !Array.isArray(progress.audit)) return []
  return progress.audit.filter(isBashAuditEvent)
}

function isBashAuditEvent(value: unknown): value is BashAuditEvent {
  if (!isRecord(value)) return false
  if (value.kind === 'registered-command') {
    return typeof value.name === 'string' && isStringArray(value.args) && typeof value.exitCode === 'number'
  }
  if (value.kind === 'system-command') {
    return (
      typeof value.name === 'string' &&
      isStringArray(value.args) &&
      typeof value.cwd === 'string' &&
      (typeof value.exitCode === 'number' || value.exitCode === null)
    )
  }
  return false
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === 'string')
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function errorCode(error: unknown): string | undefined {
  if (!isRecord(error) || typeof error.code !== 'string') return undefined
  return error.code
}

function noop(): void {}
