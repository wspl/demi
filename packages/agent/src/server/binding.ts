import { errorCode, noop } from '@demicodes/utils'
import { providerRuntime, type ProviderSelection } from '@demicodes/provider'
import type { AgentSession } from '../session/session'
import { cloneBlocks } from '../transcript/patch'
import type { ClientFrame, ServerFrame } from '../protocol/frames'
import { clientFrameSchema } from '../protocol/schemas'
import type { AgentServerTransport } from '../protocol/transport'
import type { AgentHarness, AgentNodeRecord, AgentTreeStore, ModelSwitchApply } from '../types'
import { assembleNode, type NodeDeps, type TreeContext } from '../node/assemble'
import { ROOT_POLICY } from '../node/node'
import { AgentDirectory } from '../subagent/directory'
import { LiveSession } from './live-session'
import type { SessionAttachment, SessionOwnershipRegistry } from './ownership'
import { errorDiagnostics, progressToOutput, progressToShellOutput } from './summaries'
import type { AgentTransportBinding, ProviderResolver } from './server'

export interface AgentTransportBindingOptions {
  transport: AgentServerTransport
  agent: AgentHarness<unknown>
  resolveProvider: ProviderResolver
  deps: NodeDeps<unknown>
  store: (rootSessionId: string) => AgentTreeStore<unknown>
  sessions: SessionOwnershipRegistry
}

/**
 * One attached transport: frame dispatch and the attach/detach lifecycle.
 * The root node comes from the node assembly (`node/assemble.ts`); the
 * running session itself lives in the ownership registry, not here — this
 * object is only ever the currently attached view onto it.
 */
export class AgentTransportBindingImpl implements AgentTransportBinding, SessionAttachment {
  private readonly transport: AgentServerTransport
  private readonly agent: AgentHarness<unknown>
  private readonly resolveProvider: ProviderResolver
  private readonly deps: NodeDeps<unknown>
  private readonly store: (rootSessionId: string) => AgentTreeStore<unknown>
  private readonly sessions: SessionOwnershipRegistry
  private live: LiveSession | null = null
  private unsubscribeTransport: (() => void) | null = null
  private closed = false

  constructor(options: AgentTransportBindingOptions) {
    this.transport = options.transport
    this.agent = options.agent
    this.resolveProvider = options.resolveProvider
    this.deps = options.deps
    this.store = options.store
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
    const agent = this.agent
    // The tree this root heads: its store, its directory, its frame sink, the harness profiles.
    let live: LiveSession | null = null
    const tree: TreeContext<unknown> = {
      store: this.store(agentSessionId),
      directory: new AgentDirectory<unknown>(),
      hostSessionId: agentSessionId,
      profiles: (await agent.agents?.({ state: agent.initialState(), cwd: frame.cwd })) ?? null,
      emit: (serverFrame) => live?.sink(serverFrame),
    }
    const record: AgentNodeRecord = {
      id: agentSessionId,
      parentId: null,
      description: '',
      profileName: null,
      metadata: null,
      spawnedAt: Date.now(),
      canSpawnSubagents: true,
      closedPhase: null,
      closedAt: null,
      result: null,
      failure: null,
      delivered: false,
    }
    const { node, restored } = await assembleNode(this.deps, tree, {
      record,
      cwd: frame.cwd,
      provider,
      model: frame.provider.model,
      prompt: { systemPrompt: agent.systemPrompt.bind(agent), preamble: agent.preamble?.bind(agent) },
      preambleSuffix: null,
      commands: async (state) => (await agent.commands?.({ state, cwd: frame.cwd, agentSessionId })) ?? [],
      shellEnv: {},
      policy: ROOT_POLICY,
      firstMessage: null,
      onJobsChanged: null,
    })
    tree.directory.attachRoot(node)
    live = new LiveSession(node, frame.provider.providerId)
    this.sessions.register(live)
    this.live = live
    live.attachSink((serverFrame) => this.send(serverFrame))
    // A restored root keeps its checkpoint's model; align it with the model the
    // client opened with (which may differ from when it was saved).
    if (restored) node.session.updateModel(null, frame.provider.model)

    this.sendOpenHandshake(live)
    if (restored) {
      // What the root had queued runs again; its interrupted turn is the client's
      // to resume. Children persisted by a previous process come back after the
      // open handshake, so the client sees them exactly like a replay: started +
      // transcript_reset frames, then live patches as their turns continue.
      node.continue((turn) => this.observeSessionAction(turn))
      await node.supervisor.restore()
    }
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

  private async handleShellWrite(frame: Extract<ClientFrame, { type: 'shell_write' }>): Promise<void> {
    const live = this.live
    const session = this.sessionFor('shell_write')
    if (!session || !live) return

    const environment = await live.resolveEnvironment(
      { state: session.state(), metadata: frame.metadata ?? null },
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
