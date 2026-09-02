import { errorCode, noop } from '@demicodes/utils'
import type { BashEnvironmentOptions, Host } from '@demicodes/shell'
import { providerRuntime, type ProviderSelection } from '@demicodes/provider'
import type { AgentSession } from '../session/session'
import { cloneBlocks } from '../transcript/patch'
import type { ClientFrame, ConversationSummary, ServerFrame } from '../protocol/frames'
import { clientFrameSchema } from '../protocol/schemas'
import type { AgentServerTransport } from '../protocol/transport'
import type { AgentHarness, AgentSessionStore, ModelSwitchApply } from '../types'
import { loadPersistedSession, persistedSessionCheckpoint } from '../store/session-store'
import type { BlobStore } from '../store/media'
import type { LiveSession } from './live-session'
import { assembleLiveSession } from './open-session'
import type { SessionAttachment, SessionOwnershipRegistry } from './ownership'
import { errorDiagnostics, progressToAudit, progressToOutput, progressToShellOutput, summarizeConversation } from './summaries'
import type { AgentServerSessionOptions, AgentTransportBinding, PrepareShell, ProviderResolver, ShellEnvironmentFactory } from './server'

export interface AgentTransportBindingOptions {
  transport: AgentServerTransport
  agent: AgentHarness<unknown>
  resolveProvider: ProviderResolver
  shell?: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  session?: AgentServerSessionOptions
  prepareShell: PrepareShell | null
  shellEnvironment: ShellEnvironmentFactory
  notifyParentOnIdle: boolean
  sessions: SessionOwnershipRegistry
  sessionStore: ((agentSessionId: string, host: Host) => AgentSessionStore<unknown>) | null
  blobs: BlobStore | null
}

/**
 * One attached transport: frame dispatch and the attach/detach lifecycle.
 * Session construction lives in `open-session.ts`; the running session itself
 * lives in the ownership registry, not here — this object is only ever the
 * currently attached view onto it.
 */
export class AgentTransportBindingImpl implements AgentTransportBinding, SessionAttachment {
  private readonly transport: AgentServerTransport
  private readonly agent: AgentHarness<unknown>
  private readonly resolveProvider: ProviderResolver
  private readonly shellOptions: Omit<BashEnvironmentOptions, 'host' | 'commands'>
  private readonly sessionOptions: AgentServerSessionOptions
  private readonly prepareShell: PrepareShell | null
  private readonly shellEnvironment: ShellEnvironmentFactory
  private readonly notifyParentOnIdle: boolean
  private readonly sessions: SessionOwnershipRegistry
  private readonly sessionStore: ((agentSessionId: string, host: Host) => AgentSessionStore<unknown>) | null
  private readonly blobs: BlobStore | null
  private live: LiveSession | null = null
  private unsubscribeTransport: (() => void) | null = null
  private closed = false

  constructor(options: AgentTransportBindingOptions) {
    this.transport = options.transport
    this.agent = options.agent
    this.resolveProvider = options.resolveProvider
    this.shellOptions = options.shell ?? {}
    this.sessionOptions = options.session ?? {}
    this.prepareShell = options.prepareShell
    this.shellEnvironment = options.shellEnvironment
    this.notifyParentOnIdle = options.notifyParentOnIdle
    this.sessions = options.sessions
    this.sessionStore = options.sessionStore
    this.blobs = options.blobs
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
    // The transport hands over whatever arrived on the wire; this is the
    // trust boundary, so the frame is validated before anything acts on it.
    const parsed = clientFrameSchema.safeParse(frame)
    if (!parsed.success) {
      const issue = parsed.error.issues[0]
      this.send({
        type: 'error',
        message: `Invalid client frame${issue ? `: ${issue.path.join('.')} ${issue.message}` : ''}`,
        code: 'invalid_frame',
      })
      return
    }
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

    // The session id is client-owned: it keys the persisted rows, so a reconnect
    // with the same id resumes the conversation rather than starting a new one.
    // If the session is still live in this server the running object is adopted;
    // if another connection is attached, this open takes the attachment over.
    const agentSessionId = frame.sessionId
    const adopted = await this.sessions.claim(agentSessionId, this)
    if (adopted) {
      this.live = adopted
      adopted.attachSink((serverFrame) => this.send(serverFrame))
      await this.alignProvider(adopted, frame.provider)
      this.sendOpenHandshake(adopted)
      // Live children replay their started + transcript_reset frames for this client.
      adopted.supervisor.replay()
      return
    }

    const provider = await this.createRuntime(frame.provider, agentSessionId)
    const { live, restoring } = await assembleLiveSession(
      {
        agent: this.agent,
        shellOptions: this.shellOptions,
        sessionOptions: this.sessionOptions,
        prepareShell: this.prepareShell,
        shellEnvironment: this.shellEnvironment,
        notifyParentOnIdle: this.notifyParentOnIdle,
        sessionStore: this.sessionStore,
        blobs: this.blobs,
      },
      { agentSessionId, cwd: frame.cwd, provider, selection: frame.provider },
    )
    this.sessions.register(live)
    this.live = live
    live.attachSink((serverFrame) => this.send(serverFrame))

    this.sendOpenHandshake(live)
    // Children persisted by a previous process of this parent come back after the
    // open handshake, so the client sees them exactly like a replay: started +
    // transcript_reset frames, then live patches as their interrupted turns resume.
    if (restoring) await live.supervisor.restore()
  }

  private sendOpenHandshake(live: LiveSession): void {
    this.send({ type: 'opened' })
    this.sendTranscriptReset(live.session)
    this.send({ type: 'phase', phase: live.session.phase() })
    this.send({ type: 'queue', queue: live.session.queuedMessages() })
  }

  /** Aligns an adopted live session with the model/provider this open named. */
  private async alignProvider(live: LiveSession, selection: ProviderSelection): Promise<void> {
    if (selection.providerId === live.providerId) {
      live.session.updateModel(null, selection.model)
      return
    }
    const runtime = await this.createRuntime(selection, live.agentSessionId)
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
    const next = await this.createRuntime(provider, live.agentSessionId)
    live.session.updateModel(next, provider.model, apply)
    live.providerId = provider.providerId
  }

  private async createRuntime(selection: ProviderSelection, agentSessionId: string) {
    if (selection.model.providerId !== selection.providerId) {
      throw new Error(
        `Provider selection mismatch: providerId "${selection.providerId}" does not match model providerId "${selection.model.providerId}"`,
      )
    }
    const provider = await this.resolveProvider(selection.providerId, { agentSessionId })
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
  // survives a cleared browser / a different device. Summaries read the raw
  // rows (no media rehydration — titles and timestamps need none).
  private async listConversations(cwd: string): Promise<void> {
    const host = await this.agent.host({ state: this.agent.initialState(), cwd })
    const keys = await host.store.list('agent-sessions/')
    const conversations: ConversationSummary[] = []
    for (const key of keys) {
      if (!key.endsWith('/state.json')) continue
      const id = key.slice('agent-sessions/'.length, -'/state.json'.length)
      const loaded = await loadPersistedSession(host.store, `agent-sessions/${id}`)
      if (!loaded || loaded.state.cwd !== cwd) continue
      conversations.push(summarizeConversation(id, persistedSessionCheckpoint(loaded)))
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
