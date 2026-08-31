import { bytesToBase64, isRecord, noop, safeJsonStringify } from '@demicodes/utils'
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
import type { Block, ProviderErrorDiagnostics, ToolResultContentBlock } from '@demicodes/core'
import { providerRuntime, type Provider, type ProviderSelection } from '@demicodes/provider'
import { AgentClient } from './client'
import { cloneBlocks } from './patch'
import type { ClientFrame, ConversationSummary, ServerFrame, ShellCommandStatusLike } from './frames'
import { createInProcessTransportPair, type AgentServerTransport } from './transport'
import type {
  AgentHarness,
  AgentHarnessRuntime,
  AgentSessionStore,
  AgentSessionCheckpoint,
  AgentToolInvokeContext,
  ModelSwitchApply,
  SessionEvent,
} from './types'
import type { TurnRetryPolicy } from './retry-policy'
import { createStandardAgentTools } from './tools'
import { ChildSupervisor, injectSubagentCommand } from './subagent'
import { ProviderStreamError } from './provider-stream-error'

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

export interface AgentServerOptions {
  agent: AgentHarness<unknown>
  providers: Provider[]
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
}

export interface AgentTransportBinding {
  close(): Promise<void>
}

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

export class AgentServer {
  private readonly agent: AgentHarness<unknown>
  private readonly providers: Map<string, Provider>
  private readonly shellOptions: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  private readonly sessionOptions: AgentServerSessionOptions
  private readonly prepareShell: PrepareShell | null
  private readonly notifyParentOnIdle: boolean
  private readonly bindings = new Set<AgentTransportBindingImpl>()
  private readonly sessionOwnership = new SessionOwnershipRegistry()

  constructor(options: AgentServerOptions) {
    this.agent = options.agent
    this.providers = createProviderMap(options.providers)
    this.shellOptions = options.shell ?? {}
    this.sessionOptions = options.session ?? {}
    this.prepareShell = options.prepareShell ?? null
    this.notifyParentOnIdle = options.subagents?.notifyParentOnIdle ?? true
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
      prepareShell: this.prepareShell,
      notifyParentOnIdle: this.notifyParentOnIdle,
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

/**
 * Owns every live session in this server and tracks which transport binding is
 * currently attached to each. Sessions live in the server, not in bindings: a
 * binding going away merely detaches its subscription, and opening a session
 * id that another binding is attached to takes the attachment over — the
 * running session object itself is adopted, never restarted.
 */
class SessionOwnershipRegistry {
  private readonly live = new Map<string, LiveSession>()
  private readonly attached = new Map<string, AgentTransportBindingImpl>()

  /** Detaches any other binding from the id and records this one; returns the live session to adopt. */
  async claim(sessionId: string, binding: AgentTransportBindingImpl): Promise<LiveSession | null> {
    const previous = this.attached.get(sessionId)
    if (previous && previous !== binding) await previous.handleTakeover()
    this.attached.set(sessionId, binding)
    return this.live.get(sessionId) ?? null
  }

  register(live: LiveSession): void {
    this.live.set(live.agentSessionId, live)
  }

  unregister(sessionId: string): void {
    this.live.delete(sessionId)
  }

  release(sessionId: string, binding: AgentTransportBindingImpl): void {
    if (this.attached.get(sessionId) === binding) this.attached.delete(sessionId)
  }

  sessions(): LiveSession[] {
    return [...this.live.values()]
  }

  async disposeAll(): Promise<void> {
    const sessions = this.sessions()
    this.live.clear()
    this.attached.clear()
    await Promise.all(sessions.map((live) => live.dispose()))
  }
}

interface AgentTransportBindingOptions {
  transport: AgentServerTransport
  agent: AgentHarness<unknown>
  providers: ReadonlyMap<string, Provider>
  shell?: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  session?: AgentServerSessionOptions
  prepareShell: PrepareShell | null
  notifyParentOnIdle: boolean
  sessions: SessionOwnershipRegistry
}

class AgentTransportBindingImpl implements AgentTransportBinding {
  private readonly transport: AgentServerTransport
  private readonly agent: AgentHarness<unknown>
  private readonly providers: ReadonlyMap<string, Provider>
  private readonly shellOptions: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  private readonly sessionOptions: AgentServerSessionOptions
  private readonly prepareShell: PrepareShell | null
  private readonly notifyParentOnIdle: boolean
  private readonly sessions: SessionOwnershipRegistry
  private live: LiveSession | null = null
  private unsubscribeTransport: (() => void) | null = null
  private closed = false

  constructor(options: AgentTransportBindingOptions) {
    this.transport = options.transport
    this.agent = options.agent
    this.providers = options.providers
    this.shellOptions = options.shell ?? {}
    this.sessionOptions = options.session ?? {}
    this.prepareShell = options.prepareShell
    this.notifyParentOnIdle = options.notifyParentOnIdle
    this.sessions = options.sessions
    this.unsubscribeTransport = this.transport.onFrame((frame) => {
      void this.handleFrame(frame)
    })
  }

  /**
   * Called by the ownership registry when another connection opens this session
   * id: this binding detaches — the running session is adopted by the new
   * binding, never restarted.
   */
  async handleTakeover(): Promise<void> {
    this.detach()
    this.send({ type: 'closed' })
  }

  /**
   * Transport gone (socket closed, process detaching). Sessions live in the
   * server: an in-flight turn keeps running and the session stays adoptable —
   * only the explicit `close` frame (or server shutdown) disposes it.
   */
  async close(): Promise<void> {
    if (this.closed) return
    this.closed = true
    this.detach()
    this.unsubscribeTransport?.()
    this.unsubscribeTransport = null
    this.transport.close()
  }

  private detach(): void {
    const live = this.live
    this.live = null
    if (!live) return
    live.detachSink()
    this.sessions.release(live.agentSessionId, this)
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
          this.observeSessionAction(session.send(frame.content, { id: frame.messageId, metadata: frame.metadata }))
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
          const session = this.live?.session ?? null
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
          const session = this.live?.session ?? null
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
          this.live?.session.cancelPendingSteer(frame.steerId)
          return
        }
        case 'set_provider':
          await this.setProvider(frame.provider, frame.apply)
          return
        case 'retry': {
          const session = this.sessionFor('retry')
          if (!session || this.rejectIfBusy(session, 'retry')) return
          this.observeSessionAction(session.retry({ metadata: frame.metadata }))
          return
        }
        case 'resume': {
          const session = this.sessionFor('resume')
          if (!session || this.rejectIfBusy(session, 'resume')) return
          this.observeSessionAction(session.resume({ metadata: frame.metadata }))
          return
        }
        case 'compact': {
          const session = this.sessionFor('compact')
          if (!session || this.rejectIfBusy(session, 'compact')) return
          this.observeSessionAction(session.compact({ metadata: frame.metadata }))
          return
        }
        case 'abort': {
          const session = this.sessionFor('abort')
          if (!session) return
          const result = await session.abort()
          this.send({ type: 'abort_result', result })
          return
        }
        case 'abort_subagents': {
          await this.live?.supervisor.abortAll()
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
          this.sendTranscriptReset(session)
          this.live?.supervisor.replay()
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
    if (this.live) {
      this.send({ type: 'rejected', command: 'open', reason: 'A session is already open on this connection' })
      return
    }

    const agent = this.agent
    // The session id is client-owned: it keys the checkpoint, so a reconnect with
    // the same id resumes the conversation rather than starting a new one. If the
    // session is still live in this server the running object is adopted; if
    // another connection is attached, this open takes the attachment over.
    const agentSessionId = frame.sessionId
    const adopted = await this.sessions.claim(agentSessionId, this)
    if (adopted) {
      this.live = adopted
      adopted.attachSink((serverFrame) => this.send(serverFrame))
      await this.alignProvider(adopted, frame.provider)
      this.send({ type: 'opened' })
      this.sendTranscriptReset(adopted.session)
      this.send({ type: 'phase', phase: adopted.session.phase() })
      this.send({ type: 'queue', queue: adopted.session.queuedMessages() })
      // Live children replay their started + transcript_reset frames for this client.
      adopted.supervisor.replay()
      return
    }

    const provider = await this.createRuntime(frame.provider)

    // The checkpoint lives in Host.store, so a Host is needed before the restored
    // state exists. Harnesses must tolerate host() being called with initial
    // state for store access (listConversations does the same).
    const initialState = agent.initialState()
    const provisionalHost = await agent.host({ state: initialState, cwd: frame.cwd })
    const store = new HostAgentSessionStore(provisionalHost.store, agentSessionId)
    const checkpoint = await store.loadCheckpoint()
    const restoring = checkpoint !== null && checkpoint.harnessName === agent.name

    // One live state object, shared by the harness closures (host, commands,
    // prompts) and the session itself. On restore it carries the saved state.
    const state = restoring ? structuredClone(checkpoint.state) : initialState
    const harnessContext = { state, cwd: frame.cwd }
    const harnessCommands = (await agent.commands?.(harnessContext)) ?? []
    const profiles = (await agent.agents?.(harnessContext)) ?? null
    // Deferred references into the LiveSession under construction: the closures
    // below only run once the session is live.
    let live: LiveSession | null = null
    const liveSink = (serverFrame: ServerFrame): void => {
      live?.sink(serverFrame)
    }
    // Root sessions get the subagent surface (`demi agent`); child sessions are
    // supervisor-built with a send-parent-only tree, so spawn is root-only.
    const supervisor = new ChildSupervisor<unknown>({
      agent,
      cwd: frame.cwd,
      profiles,
      parentCommands: harnessCommands,
      shellOptions: this.shellOptions,
      prepareShell: this.prepareShell,
      sessionOptions: this.sessionOptions,
      notifyParentOnIdle: this.notifyParentOnIdle,
      store: provisionalHost.store,
      emit: liveSink,
    })
    const commands = injectSubagentCommand(harnessCommands, supervisor.rootCommandNode())
    const commandRegistry = new CommandRegistry()
    for (const command of commands) commandRegistry.register(command)
    let sessionRef: AgentSession<unknown> | null = null
    const tools = createStandardAgentTools({
      environment: (ctx, handle) => {
        if (!live) throw new Error('AgentServer: session is not ready for shell access')
        return live.resolveEnvironment(ctx, handle)
      },
      scheduleYield: (ctx, durationMs) => {
        if (!sessionRef) throw new Error('AgentServer: session is not ready for yield scheduling')
        return sessionRef.scheduleYieldWakeup(durationMs, ctx.metadata)
      },
    })
    // Commands are fixed for the session's lifetime, so the rendered help is too.
    const commandsPrompt = commandRegistry.renderHelp()
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
      ? AgentSession.fromCheckpoint(
          { provider, runtime, checkpoint: { ...checkpoint, state } },
          { agentSessionId, store, ...this.sessionOptions },
        )
      : new AgentSession(
          { provider, model: frame.provider.model, cwd: frame.cwd, runtime, state },
          { agentSessionId, store, ...this.sessionOptions },
        )
    sessionRef = session
    supervisor.attachParent(session)
    live = new LiveSession({
      agentSessionId,
      session,
      supervisor,
      agent,
      commandRegistry,
      cwd: frame.cwd,
      providerId: frame.provider.providerId,
      shellOptions: this.shellOptions,
      prepareShell: this.prepareShell,
    })
    this.sessions.register(live)
    this.live = live
    live.attachSink((serverFrame) => this.send(serverFrame))
    // A resumed session restores its model from the checkpoint; align it with the
    // model the client opened with (which may differ from when it was saved).
    if (restoring) session.updateModel(null, frame.provider.model)

    this.send({ type: 'opened' })
    this.sendTranscriptReset(session)
    this.send({ type: 'phase', phase: session.phase() })
    this.send({ type: 'queue', queue: session.queuedMessages() })
    // Children persisted by a previous process of this parent come back after the
    // open handshake, so the client sees them exactly like a replay: started +
    // transcript_reset frames, then live patches as their interrupted turns resume.
    if (restoring) await supervisor.restore()
  }

  /** Aligns an adopted live session with the model/provider this open named. */
  private async alignProvider(live: LiveSession, selection: ProviderSelection): Promise<void> {
    if (selection.providerId === live.providerId) {
      live.session.updateModel(null, selection.model)
      return
    }
    const runtime = await this.createRuntime(selection)
    live.session.updateModel(runtime, selection.model)
    live.providerId = selection.providerId
  }

  private sendTranscriptReset(session: AgentSession<unknown>): void {
    const transcript = session.transcript()
    this.send({ type: 'transcript_reset', blocks: cloneBlocks(transcript.blocks), revision: transcript.revision })
  }

  private async setProvider(provider: ProviderSelection, apply?: ModelSwitchApply): Promise<void> {
    const live = this.live
    if (!live) {
      this.send({ type: 'rejected', command: 'set_provider', reason: 'No session is open on this connection' })
      return
    }
    if (provider.providerId === live.providerId) {
      // Same provider id: keep the instance and only swap the model (the provider itself
      // restarts whatever it needs to when the model id changes on the next request).
      live.session.updateModel(null, provider.model, apply)
      return
    }
    const next = await this.createRuntime(provider)
    live.session.updateModel(next, provider.model, apply)
    live.providerId = provider.providerId
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

  /** Explicit `close` frame: the session is disposed for real, not just detached. */
  private async closeSession(): Promise<void> {
    const live = this.live
    this.live = null
    if (!live) return
    this.sessions.unregister(live.agentSessionId)
    this.sessions.release(live.agentSessionId, this)
    await live.dispose()
  }

  // List the persisted conversations for a workspace (cwd), newest first, read
  // straight from Host.store — independent of any client-side state, so history
  // survives a cleared browser / a different device.
  private async listConversations(cwd: string): Promise<void> {
    const host = await this.agent.host({ state: this.agent.initialState(), cwd })
    const keys = await host.store.list('agent-sessions/')
    const conversations: ConversationSummary[] = []
    for (const key of keys) {
      if (!key.endsWith('/checkpoint.json')) continue
      const checkpoint = await host.store.readJson<AgentSessionCheckpoint<unknown>>(key)
      if (!checkpoint || checkpoint.cwd !== cwd) continue
      conversations.push(summarizeConversation(key.slice('agent-sessions/'.length, -'/checkpoint.json'.length), checkpoint))
    }
    conversations.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
    this.send({ type: 'conversations', conversations })
  }

  private async handleShellWrite(frame: Extract<ClientFrame, { type: 'shell_write' }>): Promise<void> {
    const live = this.live
    const session = this.sessionFor('shell_write')
    if (!session || !live) return

    const environment = await live.resolveEnvironment(
      {
        agentSessionId: session.id(),
        state: session.state(),
        cwd: live.cwd,
        metadata: frame.metadata ?? null,
      },
      { commandId: frame.commandId },
    )
    const result = await environment.write({
      commandId: frame.commandId,
      stdin: frame.stdin,
    })
    this.sendShellWriteResult(frame.commandId, result)
  }

  private sessionFor(command: string): AgentSession<unknown> | null {
    if (!this.live) {
      this.send({ type: 'rejected', command, reason: 'No session is open' })
      return null
    }
    return this.live.session
  }

  private rejectIfBusy(session: AgentSession<unknown>, command: string): boolean {
    const phase = session.phase()
    if (phase === 'idle') return false
    this.send({ type: 'rejected', command, reason: `Session is busy (${phase})` })
    return true
  }

  private sendShellWriteResult(commandId: string, progress: unknown): void {
    const shell = progressToShellOutput(progress)
    if (shell) {
      this.send({
        type: 'shell_output',
        shellId: shell.shellId,
        commandId: shell.commandId,
        status: shell.status,
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
    const diagnostics = errorDiagnostics(error)
    this.send({
      type: 'error',
      message: normalized.message,
      ...(code ? { code } : {}),
      ...(diagnostics ? { diagnostics } : {}),
    })
  }
}

interface LiveSessionOptions {
  agentSessionId: string
  session: AgentSession<unknown>
  supervisor: ChildSupervisor<unknown>
  agent: AgentHarness<unknown>
  commandRegistry: CommandRegistry
  cwd: string
  providerId: string
  shellOptions: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  prepareShell: PrepareShell | null
}

/**
 * One running session, owned by the server (via the ownership registry), not
 * by any transport binding. It carries everything session-scoped — the
 * AgentSession, subagent supervisor, per-Host shell environments — and emits
 * server frames through a swappable sink: the currently attached binding, or
 * nowhere while detached (turns keep running either way).
 */
class LiveSession {
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
  private readonly environmentsByHost = new Map<Host, BashEnvironment>()
  private readonly pendingEnvironmentsByHost = new Map<Host, Promise<BashEnvironment>>()
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
  ): Promise<BashEnvironment> {
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

  private async environmentForHost(host: Host, commands: CommandRegistry): Promise<BashEnvironment> {
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

  private async createEnvironment(host: Host, commands: CommandRegistry): Promise<BashEnvironment> {
    const shellOptions = this.prepareShell
      ? await this.prepareShell({
          agentSessionId: this.agentSessionId,
          host,
          commandNames: commands.list().map((command) => command.name),
          shell: this.shellOptions,
        })
      : this.shellOptions
    const environment = new BashEnvironment({ ...shellOptions, host, commands })
    return environment
  }

  private environmentForShell(shellId: string): BashEnvironment | null {
    const matches = [...this.environmentsByHost.values()].filter((environment) => environment.getShell(shellId))
    if (matches.length > 1) throw new Error(`Shell id "${shellId}" is not unique in this session`)
    return matches[0] ?? null
  }

  private environmentForCommand(commandId: string): BashEnvironment | null {
    const matches = [...this.environmentsByHost.values()].filter((environment) => environment.hasCommand(commandId))
    if (matches.length > 1) throw new Error(`Command id "${commandId}" is not unique in this session`)
    return matches[0] ?? null
  }
}

class HostAgentSessionStore<State> implements AgentSessionStore<State> {
  constructor(
    private readonly store: HostStore,
    private readonly agentSessionId: string,
  ) {}

  saveCheckpoint(checkpoint: AgentSessionCheckpoint<State>): Promise<void> {
    return this.store.writeJson(`agent-sessions/${this.agentSessionId}/checkpoint.json`, checkpoint)
  }

  loadCheckpoint(): Promise<AgentSessionCheckpoint<State> | null> {
    return this.store.readJson<AgentSessionCheckpoint<State>>(`agent-sessions/${this.agentSessionId}/checkpoint.json`)
  }
}

function summarizeConversation(id: string, checkpoint: AgentSessionCheckpoint<unknown>): ConversationSummary {
  const blocks = checkpoint.transcript.blocks
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
): { shellId: string; commandId: string; status: ShellCommandStatusLike } | null {
  if (!isRecord(progress)) return null
  if (typeof progress.shellId !== 'string' || typeof progress.commandId !== 'string') return null
  if (progress.status !== 'running' && progress.status !== 'exited' && progress.status !== 'aborted') return null
  if (!isRecord(progress.stdout) || !isRecord(progress.stderr)) return null
  const stdout = progress.stdout
  const stderr = progress.stderr
  if (
    !isShellStreamView(stdout) ||
    !isShellStreamView(stderr) ||
    typeof progress.runningMs !== 'number' ||
    typeof progress.idleMs !== 'number'
  ) {
    return null
  }
  return {
    shellId: progress.shellId,
    commandId: progress.commandId,
    status: progress as unknown as ShellCommandStatusLike,
  }
}

function isShellStreamView(value: Record<string, unknown>): boolean {
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

function errorDiagnostics(error: unknown): ProviderErrorDiagnostics | undefined {
  if (!(error instanceof ProviderStreamError)) return undefined
  return error.diagnostics
}

function createProviderMap(providers: Provider[]): Map<string, Provider> {
  const map = new Map<string, Provider>()
  for (const provider of providers) {
    if (map.has(provider.id)) throw new Error(`AgentServer: provider "${provider.id}" is already configured`)
    map.set(provider.id, provider)
  }
  return map
}
