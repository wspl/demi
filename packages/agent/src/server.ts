import { isRecord, noop, safeJsonStringify } from '@demicodes/utils'
import { AgentSession } from './session'
import {
  BashEnvironment,
  CommandRegistry,
  MAX_TIMEOUT_MS,
  heredocDelimiter,
  shellQuote,
  type BashAuditEvent,
  type BashEnvironmentOptions,
  type Host,
  type HostStore,
} from '@demicodes/shell'
import type { Block, ToolResultContentBlock } from '@demicodes/core'
import { providerRuntime, type Provider, type ProviderSelection } from '@demicodes/provider'
import { AgentClient } from './client'
import { materializeCommandBridgeShims } from './command-bridge-shim'
import { cloneBlocks } from './patch'
import type { ClientFrame, ConversationSummary, ServerFrame, ShellCommandSnapshotLike } from './frames'
import { createInProcessTransportPair, type AgentServerTransport } from './transport'
import type { AgentHarness, AgentHarnessRuntime, AgentSessionStore, AgentSessionSnapshot, SessionEvent } from './types'
import type { TurnRetryPolicy } from './retry-policy'
import { createStandardAgentTools } from './tools'

/** Session tuning forwarded to every AgentSession this server creates. */
export interface AgentServerSessionOptions {
  retry?: Partial<TurnRetryPolicy>
  compaction?: { keepRecentTokens?: number; preflightThresholdRatio?: number }
  persistIntervalMs?: number
}

/**
 * Low-level command bridge wiring on AgentServer. Prefer
 * `createLocalAgentServer` from `@demicodes/host-local` (bridge **on by default**).
 * When set: each `open()` materializes PATH shims; a Node listener must be
 * running at `socketPath` (`@demicodes/agent/command-bridge`).
 */
export interface AgentServerCommandBridgeOptions {
  /** Absolute filesystem path for the process-wide UDS endpoint. */
  socketPath: string
  /** Dispatch script body (from `@demicodes/agent/command-bridge`). */
  shimSource: string
}

export interface AgentServerOptions {
  agent: AgentHarness<unknown>
  providers: Provider[]
  shell?: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  session?: AgentServerSessionOptions
  /** Low-level; products should use `createLocalAgentServer` instead. */
  commandBridge?: AgentServerCommandBridgeOptions
}

export interface AgentTransportBinding {
  close(): Promise<void>
}

export interface CommandBridgeRunOptions {
  cwd: string
  stdin: string
  signal?: AbortSignal
}

export interface CommandBridgeResult {
  exitCode: number
  stdout: string
  stderr: string
}

export class CommandBridgeSessionNotFoundError extends Error {
  constructor(readonly agentSessionId: string) {
    super(`Command bridge: no session "${agentSessionId}" is open in this process`)
    this.name = 'CommandBridgeSessionNotFoundError'
  }
}

export class CommandBridgeTimeoutError extends Error {
  constructor(
    readonly commandId: string,
    readonly partialStdout: string,
    readonly partialStderr: string,
  ) {
    super(`Command bridge: command "${commandId}" exceeded the ${MAX_TIMEOUT_MS}ms bridge ceiling and was aborted`)
    this.name = 'CommandBridgeTimeoutError'
  }
}

export class AgentServer {
  private readonly agent: AgentHarness<unknown>
  private readonly providers: Map<string, Provider>
  private readonly shellOptions: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  private readonly sessionOptions: AgentServerSessionOptions
  private readonly commandBridgeOptions: AgentServerCommandBridgeOptions | null
  private readonly bindings = new Set<AgentTransportBindingImpl>()
  private readonly sessionOwnership = new SessionOwnershipRegistry()

  constructor(options: AgentServerOptions) {
    this.agent = options.agent
    this.providers = createProviderMap(options.providers)
    this.shellOptions = options.shell ?? {}
    this.sessionOptions = options.session ?? {}
    this.commandBridgeOptions = options.commandBridge ?? null
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
      session: this.sessionOptions,
      commandBridge: this.commandBridgeOptions,
      sessions: this.sessionOwnership,
    })
    this.bindings.add(binding)
    return binding
  }

  async close(): Promise<void> {
    const bindings = [...this.bindings]
    this.bindings.clear()
    await Promise.all(bindings.map((binding) => binding.close()))
  }

  /**
   * Runs one registered-command invocation to completion for a live session —
   * used by the command bridge shim path, not the client frame protocol.
   */
  async runCommandLine(
    agentSessionId: string,
    name: string,
    args: string[],
    opts: CommandBridgeRunOptions,
  ): Promise<CommandBridgeResult> {
    const binding = this.sessionOwnership.get(agentSessionId)
    if (!binding) throw new CommandBridgeSessionNotFoundError(agentSessionId)
    return binding.runCommandLine(name, args, opts)
  }
}

/**
 * Tracks which transport binding currently owns each client-provided session
 * id. Opening a session id that is already owned takes it over: the previous
 * binding's session is closed (flushing its snapshot) before the new open
 * proceeds, so two connections never write the same snapshot key concurrently.
 */
class SessionOwnershipRegistry {
  private readonly holders = new Map<string, AgentTransportBindingImpl>()

  async claim(sessionId: string, binding: AgentTransportBindingImpl): Promise<void> {
    const previous = this.holders.get(sessionId)
    if (previous && previous !== binding) await previous.handleTakeover()
    this.holders.set(sessionId, binding)
  }

  release(sessionId: string, binding: AgentTransportBindingImpl): void {
    if (this.holders.get(sessionId) === binding) this.holders.delete(sessionId)
  }

  get(sessionId: string): AgentTransportBindingImpl | undefined {
    return this.holders.get(sessionId)
  }
}

interface AgentTransportBindingOptions {
  transport: AgentServerTransport
  agent: AgentHarness<unknown>
  providers: ReadonlyMap<string, Provider>
  shell?: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  session?: AgentServerSessionOptions
  commandBridge: AgentServerCommandBridgeOptions | null
  sessions: SessionOwnershipRegistry
}

class AgentTransportBindingImpl implements AgentTransportBinding {
  private readonly transport: AgentServerTransport
  private readonly agent: AgentHarness<unknown>
  private readonly providers: ReadonlyMap<string, Provider>
  private readonly shellOptions: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  private readonly sessionOptions: AgentServerSessionOptions
  private readonly commandBridgeOptions: AgentServerCommandBridgeOptions | null
  private readonly sessions: SessionOwnershipRegistry
  private session: AgentSession<unknown> | null = null
  private currentAgent: AgentHarness<unknown> | null = null
  private currentEnvironment: BashEnvironment | null = null
  private currentCwd: string | null = null
  private currentProviderId: string | null = null
  private currentSessionId: string | null = null
  private unsubscribeSession: (() => void) | null = null
  private unsubscribeTransport: (() => void) | null = null
  private closed = false

  constructor(options: AgentTransportBindingOptions) {
    this.transport = options.transport
    this.agent = options.agent
    this.providers = options.providers
    this.shellOptions = options.shell ?? {}
    this.sessionOptions = options.session ?? {}
    this.commandBridgeOptions = options.commandBridge
    this.sessions = options.sessions
    this.unsubscribeTransport = this.transport.onFrame((frame) => {
      void this.handleFrame(frame)
    })
  }

  /** Called by the ownership registry when another connection opens this session id. */
  async handleTakeover(): Promise<void> {
    try {
      await this.closeSession()
    } catch (error) {
      this.sendError(error)
    }
    this.send({ type: 'closed' })
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
        case 'sync_transcript': {
          const session = this.sessionFor('sync_transcript')
          if (!session) return
          this.sendTranscriptSnapshot(session)
          return
        }
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
    // The session id is client-owned: it keys the snapshot, so a reconnect with
    // the same id resumes the conversation rather than starting a new one. If
    // another connection currently owns the id, this open takes it over.
    const agentSessionId = frame.sessionId
    await this.sessions.claim(agentSessionId, this)
    this.currentSessionId = agentSessionId

    // The snapshot lives in Host.store, so a Host is needed before the restored
    // state exists. Harnesses must tolerate host() being called with initial
    // state for store access (listConversations does the same).
    const initialState = agent.initialState()
    const provisionalHost = agent.host({ state: initialState, cwd: frame.cwd })
    const store = new HostAgentSessionStore(provisionalHost.store, agentSessionId)
    const snapshot = await store.loadSnapshot()
    const restoring = snapshot !== null && snapshot.harnessName === agent.name

    // One live state object, shared by the harness closures (host, commands,
    // prompts) and the session itself. On restore it carries the saved state.
    const state = restoring ? structuredClone(snapshot.state) : initialState
    const harnessContext = { state, cwd: frame.cwd }
    const host = restoring ? agent.host(harnessContext) : provisionalHost
    const commands = agent.commands?.(harnessContext) ?? []
    const commandRegistry = new CommandRegistry()
    for (const command of commands) commandRegistry.register(command)
    const shellOptions = this.commandBridgeOptions
      ? await this.withCommandBridgeEnv(host, agentSessionId, commandRegistry, this.commandBridgeOptions)
      : this.shellOptions
    const environment = new BashEnvironment({
      ...shellOptions,
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
    // Commands are fixed for the session's lifetime, so the rendered help is too.
    const commandsPrompt = commandRegistry.renderPrompt()
    const runtime: AgentHarnessRuntime<unknown> = {
      harnessName: agent.name,
      initialState: () => agent.initialState(),
      systemPrompt: (ctx) => agent.systemPrompt({ ...ctx, commandsPrompt }),
      preamble: (ctx) => agent.preamble?.(ctx) ?? null,
      resolveReferences: (ctx, content) => agent.resolveReferences?.(ctx, content) ?? content,
      lifecycle: (event) => agent.lifecycle?.(event),
      tools: () => tools,
    }
    const session = restoring
      ? AgentSession.fromSnapshot(
          { provider, runtime, snapshot: { ...snapshot, state } },
          { agentSessionId, store, ...this.sessionOptions },
        )
      : new AgentSession(
          { provider, model: frame.provider.model, cwd: frame.cwd, runtime, state },
          { agentSessionId, store, ...this.sessionOptions },
        )
    sessionRef = session
    this.session = session
    this.currentAgent = agent
    this.currentEnvironment = environment
    this.currentCwd = frame.cwd
    this.currentProviderId = frame.provider.providerId
    // A resumed session restores its model from the snapshot; align it with the
    // model the client opened with (which may differ from when it was saved).
    if (restoring) session.updateModel(null, frame.provider.model)
    this.unsubscribeSession = this.session.subscribe((event) => this.handleSessionEvent(event))

    this.send({ type: 'opened' })
    this.sendTranscriptSnapshot(session)
    this.send({ type: 'phase', phase: this.session.phase() })
    this.send({ type: 'queue', queue: this.session.queuedMessages() })
  }

  private sendTranscriptSnapshot(session: AgentSession<unknown>): void {
    const transcript = session.transcript()
    this.send({ type: 'transcript_snapshot', blocks: cloneBlocks(transcript.blocks), revision: transcript.revision })
  }

  private handleSessionEvent(event: SessionEvent): void {
    switch (event.type) {
      case 'transcript_changed':
        this.send({ type: 'transcript_patch', patches: event.patches, revision: event.revision })
        return
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
      case 'retry_scheduled':
        this.send({ type: 'retry_scheduled', attempt: event.attempt, delayMs: event.delayMs, code: event.code })
        return
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
      if (this.currentSessionId) {
        this.sessions.release(this.currentSessionId, this)
        this.currentSessionId = null
      }
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

  /**
   * Materializes this session's command bridge shim directory and returns
   * shell options with PATH prepended and DEMI_COMMAND_BRIDGE_SOCK set.
   */
  private async withCommandBridgeEnv(
    host: Host,
    agentSessionId: string,
    commandRegistry: CommandRegistry,
    bridge: AgentServerCommandBridgeOptions,
  ): Promise<Omit<BashEnvironmentOptions, 'host' | 'commands'>> {
    const commandNames = commandRegistry.list().map((command) => command.name)
    const shimDir = await materializeCommandBridgeShims(host, agentSessionId, commandNames, bridge.shimSource)
    const existingPath = this.shellOptions.initialEnv?.PATH
    return {
      ...this.shellOptions,
      initialEnv: {
        ...this.shellOptions.initialEnv,
        DEMI_COMMAND_BRIDGE_SOCK: bridge.socketPath,
        PATH: existingPath ? `${shimDir}:${existingPath}` : shimDir,
      },
    }
  }

  /** Backs `AgentServer.runCommandLine` — see its doc comment for the contract. */
  async runCommandLine(name: string, args: string[], opts: CommandBridgeRunOptions): Promise<CommandBridgeResult> {
    const environment = this.currentEnvironment
    const agentSessionId = this.currentSessionId
    if (!environment || !agentSessionId) throw new Error('Command bridge: session has no active shell environment')

    const words = [name, ...args].map(shellQuote).join(' ')
    let script = words
    if (opts.stdin.length > 0) {
      const delimiter = heredocDelimiter(opts.stdin)
      script = `${words} <<'${delimiter}'\n${opts.stdin}\n${delimiter}`
    }
    const cdScript = `cd ${shellQuote(opts.cwd)} && ${script}`

    const result = await environment.exec({
      agentSessionId,
      script: cdScript,
      timeoutMs: MAX_TIMEOUT_MS,
      signal: opts.signal,
    })

    if (result.status === 'exited') {
      return { exitCode: result.exitCode, stdout: result.stdout.delta, stderr: result.stderr.delta }
    }
    if (result.status === 'aborted') {
      throw new Error(`Command bridge: call for "${name}" was cancelled before it completed`)
    }
    const aborted = await environment.abort({ commandId: result.commandId })
    throw new CommandBridgeTimeoutError(
      result.commandId,
      aborted.status === 'aborted' ? aborted.stdout.delta : '',
      aborted.status === 'aborted' ? aborted.stderr.delta : '',
    )
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
  return safeJsonStringify(progress) ?? String(progress)
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
