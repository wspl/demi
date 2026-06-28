import { isRecord, noop } from '@demicodes/utils'
import { AgentSession } from './session'
import {
  BashEnvironment,
  CommandRegistry,
  type BashAuditEvent,
  type BashEnvironmentOptions,
  type HostStore,
} from '@demicodes/shell'
import type { Block, ToolResultContentBlock } from '@demicodes/core'
import { providerRuntime, type Provider, type ProviderSelection } from '@demicodes/provider'
import { AgentClient } from './client'
import { cloneBlocks, diffTranscriptBlocks } from './patch'
import type { ClientFrame, ConversationSummary, ServerFrame, ShellCommandSnapshotLike } from './frames'
import { createInProcessTransportPair, type AgentServerTransport } from './transport'
import type { AgentHarness, AgentHarnessRuntime, AgentSessionStore, AgentSessionSnapshot, SessionEvent } from './types'
import { createStandardAgentTools } from './tools'

export interface AgentServerOptions {
  agent: AgentHarness<unknown>
  providers: Provider[]
  shell?: Omit<BashEnvironmentOptions, 'host' | 'commands'>
}

export interface AgentTransportBinding {
  close(): Promise<void>
}

export class AgentServer {
  private readonly agent: AgentHarness<unknown>
  private readonly providers: Map<string, Provider>
  private readonly shellOptions: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  private readonly bindings = new Set<AgentTransportBindingImpl>()

  constructor(options: AgentServerOptions) {
    this.agent = options.agent
    this.providers = createProviderMap(options.providers)
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
      providers: this.providers,
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
  providers: ReadonlyMap<string, Provider>
  shell?: Omit<BashEnvironmentOptions, 'host' | 'commands'>
}

class AgentTransportBindingImpl implements AgentTransportBinding {
  private readonly transport: AgentServerTransport
  private readonly agent: AgentHarness<unknown>
  private readonly providers: ReadonlyMap<string, Provider>
  private readonly shellOptions: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  private session: AgentSession<unknown> | null = null
  private currentAgent: AgentHarness<unknown> | null = null
  private currentEnvironment: BashEnvironment | null = null
  private currentCwd: string | null = null
  private currentProviderId: string | null = null
  private unsubscribeSession: (() => void) | null = null
  private lastTranscriptBlocks: Block[] = []
  private unsubscribeTransport: (() => void) | null = null
  private closed = false

  constructor(options: AgentTransportBindingOptions) {
    this.transport = options.transport
    this.agent = options.agent
    this.providers = options.providers
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
          this.observeSessionAction(session.send(frame.content, { id: frame.messageId }))
          return
        }
        case 'dequeue_message': {
          const session = this.sessionFor('dequeue_message')
          if (!session) return
          session.dequeueMessage(frame.messageId)
          return
        }
        case 'send_queued_message': {
          const session = this.sessionFor('send_queued_message')
          if (!session) return
          session.sendQueuedMessage(frame.messageId)
          return
        }
        case 'steer_queued_message': {
          const session = this.session
          if (!session) {
            this.send({ type: 'steer_result', steerId: frame.steerId, status: 'rejected', reason: 'No session is open on this connection' })
            return
          }
          try {
            const accepted = await session.steerQueuedMessage(frame.messageId, { id: frame.steerId })
            if (accepted) this.send({ type: 'steer_result', steerId: frame.steerId, status: 'accepted' })
            else {
              this.send({
                type: 'steer_result',
                steerId: frame.steerId,
                status: 'rejected',
                reason: 'Queued message not found',
              })
            }
          } catch (error) {
            const message = error instanceof Error ? error.message : String(error)
            this.send({ type: 'steer_result', steerId: frame.steerId, status: 'rejected', reason: message })
          }
          return
        }
        case 'clear_message_queue': {
          const session = this.sessionFor('clear_message_queue')
          if (!session) return
          session.clearMessageQueue()
          return
        }
        case 'steer': {
          const session = this.session
          if (!session) {
            this.send({ type: 'steer_result', steerId: frame.steerId, status: 'rejected', reason: 'No session is open on this connection' })
            return
          }
          try {
            await session.steer(frame.content, { id: frame.steerId })
            this.send({ type: 'steer_result', steerId: frame.steerId, status: 'accepted' })
          } catch (error) {
            const message = error instanceof Error ? error.message : String(error)
            this.send({ type: 'steer_result', steerId: frame.steerId, status: 'rejected', reason: message })
          }
          return
        }
        case 'cancel_pending_steer': {
          this.session?.cancelPendingSteer(frame.steerId)
          return
        }
        case 'set_provider':
          await this.setProvider(frame.provider)
          return
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
          const result = await session.abort()
          this.send({ type: 'abort_result', result })
          return
        }
        case 'shell_write':
          await this.handleShellWrite(frame)
          return
        case 'list_conversations':
          await this.listConversations(frame.cwd)
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
    const provider = await this.createRuntime(frame.provider)
    const state = agent.initialState()
    const harnessContext = { state, cwd: frame.cwd }
    const commands = agent.commands?.(harnessContext) ?? []
    const commandRegistry = new CommandRegistry()
    for (const command of commands) commandRegistry.register(command)
    const host = agent.host(harnessContext)
    const environment = new BashEnvironment({
      ...this.shellOptions,
      host,
      commands: commandRegistry,
    })
    let sessionRef: AgentSession<unknown> | null = null
    const tools = createStandardAgentTools({
      environment,
      scheduleYield: (_ctx, durationMs) => {
        if (!sessionRef) throw new Error('AgentServer: session is not ready for yield scheduling')
        return sessionRef.scheduleYieldWakeup(durationMs)
      },
    })
    const runtime: AgentHarnessRuntime<unknown> = {
      harnessName: agent.name,
      initialState: () => state,
      systemPrompt: (ctx) => agent.systemPrompt(ctx),
      preamble: (ctx) => agent.preamble?.(ctx) ?? null,
      resolveReferences: (ctx, content) => agent.resolveReferences?.(ctx, content) ?? content,
      lifecycle: (event) => agent.lifecycle?.(event),
      tools: () => tools,
    }
    // The session id is client-owned: it keys the snapshot, so a reconnect with
    // the same id resumes the conversation rather than starting a new one.
    const agentSessionId = frame.sessionId
    const store = new HostAgentSessionStore(host.store, agentSessionId)
    const snapshot = await store.loadSnapshot()
    const session =
      snapshot && snapshot.harnessName === runtime.harnessName
        ? AgentSession.fromSnapshot({ provider, runtime, snapshot }, { agentSessionId, store })
        : new AgentSession({ provider, model: frame.provider.model, cwd: frame.cwd, runtime, state }, { agentSessionId, store })
    sessionRef = session
    this.session = session
    this.currentAgent = agent
    this.currentEnvironment = environment
    this.currentCwd = frame.cwd
    this.currentProviderId = frame.provider.providerId
    // A resumed session restores its model from the snapshot; align it with the
    // model the client opened with (which may differ from when it was saved).
    if (snapshot) await session.updateModel(null, frame.provider.model)
    const restoredBlocks = session.transcript().blocks
    this.lastTranscriptBlocks = cloneBlocks(restoredBlocks)
    this.unsubscribeSession = this.session.subscribe((event) => this.handleSessionEvent(event))

    this.send({ type: 'opened' })
    this.send({ type: 'transcript_snapshot', blocks: cloneBlocks(restoredBlocks) })
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
        this.sendToolProgress(event.toolCallId, event.toolName, event.progress)
        return
      }
      case 'error':
        this.sendError(event.error)
        return
    }
  }

  private async setProvider(provider: ProviderSelection): Promise<void> {
    const session = this.session
    if (!session) {
      this.send({ type: 'rejected', command: 'set_provider', reason: 'No session is open on this connection' })
      return
    }
    if (provider.providerId === this.currentProviderId) {
      // Same provider id: keep the instance and only swap the model (the provider itself
      // restarts whatever it needs to when the model id changes on the next request).
      await session.updateModel(null, provider.model)
      return
    }
    const next = await this.createRuntime(provider)
    await session.updateModel(next, provider.model)
    this.currentProviderId = provider.providerId
  }

  private async createRuntime(selection: ProviderSelection) {
    if (selection.model.providerId !== selection.providerId) {
      throw new Error(
        `Provider selection mismatch: providerId "${selection.providerId}" does not match model providerId "${selection.model.providerId}"`,
      )
    }
    const provider = this.providers.get(selection.providerId)
    if (!provider) throw new Error(`Provider "${selection.providerId}" is not available`)
    return providerRuntime(provider, selection)
  }

  private async closeSession(): Promise<void> {
    const session = this.session
    const agent = this.currentAgent
    const environment = this.currentEnvironment
    const cwd = this.currentCwd

    try {
      if (session) await session.dispose()
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
      this.currentProviderId = null
      this.lastTranscriptBlocks = []
    }
  }

  // List the persisted conversations for a workspace (cwd), newest first, read
  // straight from Host.store — independent of any client-side state, so history
  // survives a cleared browser / a different device.
  private async listConversations(cwd: string): Promise<void> {
    const host = this.agent.host({ state: this.agent.initialState(), cwd })
    const keys = await host.store.list('agent-sessions/')
    const conversations: ConversationSummary[] = []
    for (const key of keys) {
      if (!key.endsWith('/snapshot.json')) continue
      const snapshot = await host.store.readJson<AgentSessionSnapshot<unknown>>(key)
      if (!snapshot || snapshot.cwd !== cwd) continue
      conversations.push(summarizeConversation(key.slice('agent-sessions/'.length, -'/snapshot.json'.length), snapshot))
    }
    conversations.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
    this.send({ type: 'conversations', conversations })
  }

  private async handleShellWrite(frame: Extract<ClientFrame, { type: 'shell_write' }>): Promise<void> {
    const session = this.sessionFor('shell_write')
    if (!session || !this.currentEnvironment || !this.currentCwd) return

    const result = await this.currentEnvironment.write({
      commandId: frame.commandId,
      stdin: frame.stdin,
    })
    this.sendShellWriteResult(frame.commandId, result)
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

  private sendToolProgress(toolCallId: string, toolName: string, progress: unknown): void {
    const output = progressToOutput(progress)
    this.send({ type: 'tool_progress', toolUseId: toolCallId, output })
    const shell = toolName === 'shell_status' ? null : progressToShellOutput(progress)
    if (shell) {
      this.send({
        type: 'shell_output',
        shellId: shell.shellId,
        commandId: shell.commandId,
        snapshot: shell.snapshot,
      })
    }
    const audit = progressToAudit(progress)
    if (audit.length > 0) this.send({ type: 'audit', events: audit })
  }

  private sendShellWriteResult(commandId: string, progress: unknown): void {
    const shell = progressToShellOutput(progress)
    if (shell) {
      this.send({
        type: 'shell_output',
        shellId: shell.shellId,
        commandId: shell.commandId,
        snapshot: shell.snapshot,
      })
    }
    const audit = progressToAudit(progress)
    if (audit.length > 0) this.send({ type: 'audit', events: audit })
    this.send({ type: 'shell_write_result', commandId, output: progressToOutput(progress) })
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

class HostAgentSessionStore<State> implements AgentSessionStore<State> {
  constructor(
    private readonly store: HostStore,
    private readonly agentSessionId: string,
  ) {}

  saveSnapshot(snapshot: AgentSessionSnapshot<State>): Promise<void> {
    return this.store.writeJson(`agent-sessions/${this.agentSessionId}/snapshot.json`, snapshot)
  }

  loadSnapshot(): Promise<AgentSessionSnapshot<State> | null> {
    return this.store.readJson<AgentSessionSnapshot<State>>(`agent-sessions/${this.agentSessionId}/snapshot.json`)
  }
}

function summarizeConversation(id: string, snapshot: AgentSessionSnapshot<unknown>): ConversationSummary {
  const blocks = snapshot.transcript.blocks
  const first = blocks[0]
  const last = blocks[blocks.length - 1]
  return {
    id,
    title: conversationTitle(blocks),
    createdAt: first?.createdAt ?? '',
    updatedAt: last?.createdAt ?? first?.createdAt ?? '',
  }
}

function conversationTitle(blocks: Block[]): string {
  const user = blocks.find((block): block is Extract<Block, { type: 'user' }> => block.type === 'user')
  const text = user?.content.find((item): item is { type: 'text'; text: string } => item.type === 'text')?.text
  const title = (text ?? '').replace(/\s+/g, ' ').trim()
  return title ? title.slice(0, 80) : 'Untitled conversation'
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

function progressToShellOutput(
  progress: unknown,
): { shellId: string; commandId: string; snapshot: ShellCommandSnapshotLike } | null {
  if (!isRecord(progress)) return null
  if (typeof progress.shellId !== 'string' || typeof progress.commandId !== 'string') return null
  if (progress.status !== 'running' && progress.status !== 'exited' && progress.status !== 'aborted') return null
  if (!isRecord(progress.stdout) || !isRecord(progress.stderr)) return null
  const stdout = progress.stdout
  const stderr = progress.stderr
  if (
    !isStreamArtifact(stdout) ||
    !isStreamArtifact(stderr) ||
    typeof progress.runningMs !== 'number' ||
    typeof progress.idleMs !== 'number'
  ) {
    return null
  }
  return {
    shellId: progress.shellId,
    commandId: progress.commandId,
    snapshot: progress as unknown as ShellCommandSnapshotLike,
  }
}

function isStreamArtifact(value: Record<string, unknown>): boolean {
  return (
    typeof value.path === 'string' &&
    typeof value.offset === 'number' &&
    typeof value.delta === 'string' &&
    typeof value.tail === 'string' &&
    typeof value.bytes === 'number' &&
    typeof value.truncated === 'boolean'
  )
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
  if (value.kind === 'portable-command') {
    return (
      typeof value.name === 'string' &&
      isStringArray(value.args) &&
      typeof value.cwd === 'string' &&
      typeof value.exitCode === 'number'
    )
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

function errorCode(error: unknown): string | undefined {
  if (!isRecord(error) || typeof error.code !== 'string') return undefined
  return error.code
}

function createProviderMap(providers: Provider[]): Map<string, Provider> {
  const map = new Map<string, Provider>()
  for (const provider of providers) {
    if (map.has(provider.id)) throw new Error(`AgentServer: provider "${provider.id}" is already configured`)
    map.set(provider.id, provider)
  }
  return map
}
