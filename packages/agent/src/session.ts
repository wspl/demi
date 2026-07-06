import { AbortError, abortable, asError, createId, isAbortError, noop, throwIfAborted, truncate } from '@demicodes/utils'
import type { ModelSelection, QueuedMessage, SessionPhase, UserContentBlock } from '@demicodes/core'
import type { AgentProvider, InferenceItem, InferenceRequest, ProviderEvent, ProviderRun } from '@demicodes/provider'
import { Transcript, type TranscriptOptions } from './transcript'
import { YieldScheduler } from './yield-scheduler'
import { PendingSteerQueue, type PendingSteer } from './pending-steer-queue'
import { CompactionController, type CompactionHost } from './compaction-controller'
import { ProviderStreamError } from './provider-stream-error'
import { ProviderTurnLoop, type ProviderTurnLoopHost } from './provider-turn-loop'
import type {
  AgentHarnessRuntime,
  AgentSessionOptions,
  AgentSessionParams,
  AgentSessionRestoreParams,
  AgentSessionStore,
  AgentTool,
  AgentToolInvokeResult,
  AbortResult,
  AbortTarget,
  ExternalMutationReservation,
  SessionEvent,
  SessionEventListener,
} from './types'

const DEFAULT_KEEP_RECENT_TOKENS = 4_000
const DEFAULT_PREFLIGHT_THRESHOLD_RATIO = 0.8
const DEFAULT_PERSIST_INTERVAL_MS = 1_000

type PendingAction =
  | {
      type: 'send'
      id: string
      content: UserContentBlock[]
      // Internal yield-wakeup sends carry hidden: true so the new turn's user block is replayed
      // to the model but never rendered. Real user sends leave it unset.
      hidden?: boolean
      resolve: () => void
      reject: (error: unknown) => void
    }
  | { type: 'retry'; resolve: () => void; reject: (error: unknown) => void }
  | { type: 'resume'; resolve: () => void; reject: (error: unknown) => void }
  | { type: 'compact'; resolve: () => void; reject: (error: unknown) => void }

type PendingSendAction = Extract<PendingAction, { type: 'send' }>

export type ActiveTurnPhase = 'provider_streaming' | 'tool_executing' | 'compacting' | 'finalizing'

interface TakenQueuedSend {
  message: QueuedMessage
  messageIndex: number
  action: PendingSendAction | null
  actionIndex: number
}

export class AgentSession<State> {
  private provider: AgentProvider
  private model: ModelSelection
  private pendingModelSwitch: { provider: AgentProvider | null; model: ModelSelection } | null = null
  private readonly cwd: string
  private readonly runtime: AgentHarnessRuntime<State>
  private readonly agentSessionId: string
  private readonly idFactory: () => string
  private readonly store?: AgentSessionStore<State>
  private readonly compactionKeepRecentTokens: number
  private readonly compactionThresholdRatio: number
  private readonly listeners = new Set<SessionEventListener>()
  private readonly pendingActions: PendingAction[] = []
  private readonly queued: QueuedMessage[] = []

  private readonly transcriptLog: Transcript
  private readonly agentState: State
  private currentPhase: SessionPhase = 'idle'
  private workerRunning = false
  private externalMutationReserved = false
  private currentAbortController: AbortController | null = null
  private activeTurnId: string | null = null
  private activeTurnPhase: ActiveTurnPhase | null = null
  private activeProviderRun: ProviderRun | null = null
  private readonly steerQueue = new PendingSteerQueue()
  private readonly yields: YieldScheduler
  private readonly compaction: CompactionController
  private readonly turnLoop: ProviderTurnLoop<State>
  private abortRecorded = false
  private idleResolvers: Array<() => void> = []
  private readonly persistIntervalMs: number
  private persistTimer: ReturnType<typeof setTimeout> | null = null
  private persistDirty = false

  static fromSnapshot<State>(
    params: AgentSessionRestoreParams<State>,
    options: AgentSessionOptions<State> = {},
  ): AgentSession<State> {
    if (params.snapshot.harnessName !== params.runtime.harnessName) {
      throw new Error(
        `AgentSession: snapshot harness "${params.snapshot.harnessName}" does not match "${params.runtime.harnessName}"`,
      )
    }
    const snapshot = structuredClone(params.snapshot)
    return new AgentSession(
      {
        provider: params.provider,
        model: snapshot.model,
        cwd: snapshot.cwd,
        runtime: params.runtime,
        transcript: snapshot.transcript,
        state: snapshot.state,
      },
      options,
    )
  }

  constructor(params: AgentSessionParams<State>, options: AgentSessionOptions<State> = {}) {
    this.provider = params.provider
    this.model = params.model
    this.cwd = params.cwd
    this.runtime = params.runtime
    this.agentState = params.state === undefined ? params.runtime.initialState() : structuredClone(params.state)
    this.agentSessionId = options.agentSessionId ?? createId()
    this.idFactory = options.idFactory ?? createId
    this.yields = new YieldScheduler(this.idFactory, (wakeupId) => {
      void this.deliverYieldWakeup(wakeupId)
    })
    this.store = options.store
    this.persistIntervalMs = options.persistIntervalMs ?? DEFAULT_PERSIST_INTERVAL_MS
    this.compactionKeepRecentTokens = options.compaction?.keepRecentTokens ?? DEFAULT_KEEP_RECENT_TOKENS
    this.compactionThresholdRatio =
      options.compaction?.preflightThresholdRatio ?? DEFAULT_PREFLIGHT_THRESHOLD_RATIO

    const transcriptOptions: TranscriptOptions = {
      idFactory: this.idFactory,
      now: options.now,
    }
    this.transcriptLog =
      params.transcript instanceof Transcript
        ? params.transcript
        : new Transcript(params.transcript?.blocks ?? [], transcriptOptions)

    const self = this
    const compactionHost: CompactionHost = {
      get transcript() {
        return self.transcriptLog
      },
      get model() {
        return self.model
      },
      get provider() {
        return self.provider
      },
      get keepRecentTokens() {
        return self.compactionKeepRecentTokens
      },
      get sessionId() {
        return self.agentSessionId
      },
      get cwd() {
        return self.cwd
      },
      get thresholdRatio() {
        return self.compactionThresholdRatio
      },
      nextRequestId: () => self.idFactory(),
      currentTurnId: () => self.currentTurnId(),
      currentSignal: () => self.currentSignal(),
      streamProvider: (request, run) => self.providerEvents(request, run),
      commitTranscript: () => self.commitTranscript(),
      runWithCompactingPhase: (fn) => self.runWithCompactingPhase(fn),
    }
    this.compaction = new CompactionController(compactionHost)

    const turnLoopHost: ProviderTurnLoopHost<State> = {
      get transcript() {
        return self.transcriptLog
      },
      get model() {
        return self.model
      },
      get provider() {
        return self.provider
      },
      get runtime() {
        return self.runtime
      },
      get agentSessionId() {
        return self.agentSessionId
      },
      get cwd() {
        return self.cwd
      },
      get agentState() {
        return self.agentState
      },
      get thresholdRatio() {
        return self.compactionThresholdRatio
      },
      get steerContinuationCount() {
        return self.steerQueue.continuationCount
      },
      currentSignal: () => self.currentSignal(),
      currentTurnId: () => self.currentTurnId(),
      nextRequestId: () => self.idFactory(),
      promptContext: () => self.promptContext(),
      getActiveTurnPhase: () => self.activeTurnPhase,
      setActiveTurnPhase: (phase) => {
        self.activeTurnPhase = phase
      },
      getActiveProviderRun: () => self.activeProviderRun,
      setActiveProviderRun: (run) => {
        self.activeProviderRun = run
      },
      streamProvider: (request, run) => self.providerEvents(request, run),
      runCompaction: () => self.compaction.run(),
      runWithCompactingPhase: (fn) => self.runWithCompactingPhase(fn),
      commitTranscript: () => self.commitTranscript(),
      emit: (event) => self.emit(event),
      materializeSteersArrivedSince: (count) => self.materializePendingSteersArrivedSince(count),
    }
    this.turnLoop = new ProviderTurnLoop(turnLoopHost)
  }

  send(content: UserContentBlock[], options: { id?: string } = {}): Promise<void> {
    const id = options.id ?? this.idFactory()
    return this.enqueue({
      type: 'send',
      id,
      content,
      resolve: noop,
      reject: noop,
    })
  }

  retry(): Promise<void> {
    return this.enqueue({ type: 'retry', resolve: noop, reject: noop })
  }

  resume(): Promise<void> {
    return this.enqueue({ type: 'resume', resolve: noop, reject: noop })
  }

  compact(): Promise<void> {
    return this.enqueue({ type: 'compact', resolve: noop, reject: noop })
  }

  async steer(content: UserContentBlock[], options: { id?: string } = {}): Promise<void> {
    if (this.externalMutationReserved) {
      throw new Error('AgentSession: cannot steer while external mutation is reserved')
    }

    this.steerDelivery()
    const steerId = options.id ?? this.idFactory()
    const turnId = this.currentTurnId()
    const signal = this.currentSignal()
    const resolvedContent = await this.resolveReferences(content)
    throwIfAborted(signal)

    const deliveryAfterResolve = this.steerDelivery()
    if (this.activeTurnId !== turnId) throw new Error('AgentSession: active turn changed before steer could be accepted')

    let blockId: string | undefined
    if (deliveryAfterResolve.type === 'provider') {
      if (this.steerQueue.takeCanceled(steerId)) return
      blockId = steerId
      try {
        await deliveryAfterResolve.run.steer({
          id: blockId,
          sessionId: this.agentSessionId,
          turnId,
          content: resolvedContent,
        })
      } catch (error) {
        const normalized = asError(error)
        this.emit({ type: 'error', error: normalized })
        throw normalized
      }
    }
    if (this.activeTurnId !== turnId) throw new Error('AgentSession: active turn changed before steer could be accepted')
    if (deliveryAfterResolve.type === 'provider') {
      this.transcriptLog.pushSteer(turnId, this.model, resolvedContent, blockId)
      await this.commitTranscript()
      return
    }

    if (this.steerQueue.takeCanceled(steerId)) return
    this.steerQueue.add({
      id: steerId,
      turnId,
      model: this.model,
      content: resolvedContent,
    })
    // Transcript-backed steering is materialized after the current sampling/tool boundary,
    // matching Codex pending input semantics.
  }

  cancelPendingSteer(id: string): boolean {
    if (this.steerQueue.removePending(id)) return true
    if (this.activeTurnId && this.currentAbortController && this.activeTurnPhase !== 'finalizing') {
      this.steerQueue.markCanceled(id)
      return true
    }
    return false
  }

  /**
   * Switches the model (and optionally the provider) for the rest of the session. Queued as an
   * action so it lands at a turn boundary, never mid-tool-continuation; the next turn uses it.
   * A non-null `provider` replaces the current one (its predecessor is disposed); pass null to
   * keep the same provider instance and only change the model.
   */
  /**
   * Records a model/provider switch. It is applied at the next turn's preflight (see
   * applyPendingModelSwitch): if the new model can't hold the history, compaction runs there
   * with the current (pre-switch) model first, then the swap happens. Recording is cheap and
   * non-blocking so it never holds up an in-flight turn or other frames.
   */
  updateModel(provider: AgentProvider | null, model: ModelSelection): void {
    this.pendingModelSwitch = { provider, model }
  }

  async abort(): Promise<AbortResult> {
    if (this.currentAbortController && !this.currentAbortController.signal.aborted) {
      const target = this.activeAbortTarget()
      this.currentAbortController.abort()
      await this.recordAbort()
      return { aborted: true, target, canAbortAgain: this.canAbortAgain() }
    }

    const queuedTarget = this.abortQueuedAction()
    if (queuedTarget) return { aborted: true, target: queuedTarget, canAbortAgain: this.canAbortAgain() }

    if (this.yields.cancelOne()) {
      return { aborted: true, target: 'pending_yield_wakeup', canAbortAgain: this.canAbortAgain() }
    }

    return { aborted: false, target: null, canAbortAgain: false }
  }

  scheduleYieldWakeup(durationMs: number): AgentToolInvokeResult {
    const wakeupId = this.yields.schedule(durationMs)
    return {
      output: [
        {
          type: 'text',
          text: [`yield scheduled`, `wakeupId: ${wakeupId}`, `durationMs: ${durationMs}`].join('\n'),
        },
      ],
      metadata: {
        kind: 'yield_wakeup',
        wakeupId,
        durationMs,
      },
      stopAfterToolResult: true,
    }
  }

  waitUntilDone(): Promise<void> {
    if (!this.workerRunning && this.pendingActions.length === 0) return Promise.resolve()
    return new Promise((resolve) => {
      this.idleResolvers.push(resolve)
    })
  }

  /**
   * Tears the session down: aborts any in-flight turn and releases provider-held resources
   * (e.g. a long-lived CLI subprocess). Called when the owning connection closes.
   */
  async dispose(): Promise<void> {
    await this.abort()
    this.clearPendingActions()
    this.yields.clear()
    await this.flushPersist().catch(noop)
    const pending = this.pendingModelSwitch?.provider ?? null
    this.pendingModelSwitch = null
    await this.provider.dispose?.()
    if (pending && pending !== this.provider) await pending.dispose?.()
  }

  transcript(): Transcript {
    return this.transcriptLog
  }

  state(): State {
    return this.agentState
  }

  id(): string {
    return this.agentSessionId
  }

  phase(): SessionPhase {
    return this.currentPhase
  }

  queuedMessages(): QueuedMessage[] {
    return this.queued.map((message) => ({ ...message, content: [...message.content] }))
  }

  dequeueMessage(id: string): boolean {
    const queuedIndex = this.queued.findIndex((message) => message.id === id)
    if (queuedIndex === -1) return false

    this.queued.splice(queuedIndex, 1)
    const action = this.removePendingSend(id)
    action?.resolve()
    this.emitQueue()
    return true
  }

  sendQueuedMessage(id: string): boolean {
    const queuedIndex = this.queued.findIndex((message) => message.id === id)
    if (queuedIndex === -1) return false

    if (queuedIndex > 0) {
      const [message] = this.queued.splice(queuedIndex, 1)
      if (message) this.queued.unshift(message)
    }

    const actionIndex = this.pendingActions.findIndex((action) => action.type === 'send' && action.id === id)
    if (actionIndex !== -1) {
      const [action] = this.pendingActions.splice(actionIndex, 1)
      if (action) {
        const insertionIndex = this.pendingActions.findIndex((candidate) => candidate.type === 'send')
        if (insertionIndex === -1) this.pendingActions.push(action)
        else this.pendingActions.splice(insertionIndex, 0, action)
      }
    }

    this.emitQueue()
    this.kickWorker()
    return true
  }

  async steerQueuedMessage(id: string, options: { id?: string } = {}): Promise<boolean> {
    const queued = this.takeQueuedSend(id)
    if (!queued) return false

    try {
      await this.steer(queued.message.content, options)
    } catch (error) {
      this.restoreQueuedSend(queued)
      throw error
    }

    queued.action?.resolve()
    return true
  }

  clearMessageQueue(): number {
    if (this.queued.length === 0) return 0

    const queuedIds = new Set(this.queued.map((message) => message.id))
    const clearedCount = queuedIds.size
    this.queued.splice(0)

    for (let index = 0; index < this.pendingActions.length; ) {
      const action = this.pendingActions[index]
      if (action?.type === 'send' && queuedIds.has(action.id)) {
        this.pendingActions.splice(index, 1)
        action.resolve()
        continue
      }
      index += 1
    }

    this.emitQueue()
    return clearedCount
  }

  subscribe(listener: SessionEventListener): () => void {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  reserveMutation(): ExternalMutationReservation {
    if (this.workerRunning || this.pendingActions.length > 0 || this.externalMutationReserved) {
      throw new Error('AgentSession: cannot reserve mutation while session is busy')
    }
    this.externalMutationReserved = true
    let released = false
    return {
      release: () => {
        if (released) return
        released = true
        this.externalMutationReserved = false
      },
    }
  }

  private activeAbortTarget(): AbortTarget {
    switch (this.activeTurnPhase) {
      case 'provider_streaming':
        return 'active_provider_stream'
      case 'tool_executing':
        return 'active_tool'
      case 'compacting':
        return 'active_compaction'
      case 'finalizing':
      case null:
        return 'active_turn'
    }
  }

  private abortQueuedAction(): AbortTarget | null {
    const action = this.pendingActions.shift()
    if (!action) return null

    if (action.type === 'send') {
      this.removeQueuedMessage(action.id)
      action.resolve()
      return 'queued_message'
    }
    action.resolve()
    return 'queued_action'
  }

  private canAbortAgain(): boolean {
    if (this.currentAbortController && !this.currentAbortController.signal.aborted) return true
    return this.pendingActions.length > 0 || this.yields.hasPending
  }

  private clearPendingActions(): void {
    for (const action of this.pendingActions.splice(0)) {
      if (action.type === 'send') {
        this.removeQueuedMessage(action.id)
      }
      action.resolve()
    }
  }

  private async deliverYieldWakeup(wakeupId: string): Promise<void> {
    if (!this.yields.take(wakeupId)) return

    const content: UserContentBlock[] = [
      {
        type: 'text',
        text:
          'Scheduled yield wakeup fired. Continue the previous work and inspect any running command with shell_status when needed.',
      },
    ]

    // Active session: interject the hidden wakeup as a steer into the current turn (never queued).
    if (this.canAcceptInternalSteer()) {
      try {
        await this.steerInternal(content, wakeupId, true)
        return
      } catch (error) {
        this.emit({ type: 'error', error: asError(error) })
      }
    }

    // Idle session: deliver the hidden wakeup as a normal send that starts a new turn.
    this.enqueueHiddenSend(content)
  }

  private canAcceptInternalSteer(): boolean {
    try {
      this.steerDelivery()
      return true
    } catch {
      return false
    }
  }

  private async steerInternal(content: UserContentBlock[], id: string, hidden = false): Promise<void> {
    const delivery = this.steerDelivery()
    const turnId = this.currentTurnId()
    if (delivery.type === 'provider') {
      await delivery.run.steer({
        id,
        sessionId: this.agentSessionId,
        turnId,
        content,
      })
      this.transcriptLog.pushSteer(turnId, this.model, content, id, hidden)
      await this.commitTranscript()
      return
    }

    this.steerQueue.add({
      id,
      turnId,
      model: this.model,
      content,
      hidden,
    })
  }

  private enqueueHiddenSend(content: UserContentBlock[]): void {
    void this.enqueue({
      type: 'send',
      id: this.idFactory(),
      content,
      hidden: true,
      resolve: noop,
      reject: noop,
    }).catch(noop)
  }

  private steerDelivery():
    | { type: 'provider'; run: ProviderRun & { steer: NonNullable<ProviderRun['steer']> } }
    | { type: 'next_provider_continuation' } {
    if (!this.activeTurnId || !this.currentAbortController || !this.activeTurnPhase) {
      throw new Error('AgentSession: no active turn to steer')
    }
    if (this.currentAbortController.signal.aborted) throw new Error('AgentSession: active turn is aborted')
    if (this.activeTurnPhase === 'compacting' || this.activeTurnPhase === 'finalizing') {
      throw new Error(`AgentSession: active turn cannot accept steering while ${this.activeTurnPhase}`)
    }
    const run = this.activeProviderRun
    if (run?.steer) {
      return { type: 'provider', run: run as ProviderRun & { steer: NonNullable<ProviderRun['steer']> } }
    }
    if (this.activeTurnPhase === 'tool_executing' || this.activeTurnPhase === 'provider_streaming') {
      return { type: 'next_provider_continuation' }
    }
    throw new Error('AgentSession: active turn cannot accept steering now')
  }

  private enqueue(action: PendingAction): Promise<void> {
    if (this.externalMutationReserved) {
      return Promise.reject(new Error('AgentSession: cannot enqueue action while external mutation is reserved'))
    }

    return new Promise((resolve, reject) => {
      action.resolve = resolve
      action.reject = reject

      if (action.type === 'send' && (this.workerRunning || this.pendingActions.length > 0)) {
        this.queued.push({
          id: action.id,
          text: textContentSummary(action.content),
          content: action.content,
        })
        this.emitQueue()
      }

      this.pendingActions.push(action)
      this.kickWorker()
    })
  }

  private kickWorker(): void {
    if (this.workerRunning) return
    this.workerRunning = true
    void this.runWorker()
  }

  private async runWorker(): Promise<void> {
    try {
      while (this.pendingActions.length > 0) {
        const action = this.pendingActions.shift()
        if (!action) continue

        if (action.type === 'send') this.removeQueuedMessage(action.id)

        this.currentAbortController = new AbortController()
        this.activeTurnId = action.type === 'send' ? action.id : this.idFactory()
        this.abortRecorded = false

        try {
          await this.executeAction(action)
          await this.flushPersist()
          action.resolve()
        } catch (error) {
          if (isAbortError(error)) {
            await this.recordAbort()
            await this.flushPersist().catch((flushError: unknown) => {
              this.emit({ type: 'error', error: asError(flushError) })
            })
            action.resolve()
          } else {
            const normalized = asError(error)
            try {
              await this.materializePendingSteersForCurrentTurn()
            } catch (materializeError) {
              this.emit({ type: 'error', error: asError(materializeError) })
            }
            await this.flushPersist().catch((flushError: unknown) => {
              this.emit({ type: 'error', error: asError(flushError) })
            })
            this.emit({ type: 'error', error: normalized })
            action.reject(normalized)
          }
        } finally {
          this.discardPendingSteersForCurrentTurn()
          this.activeTurnPhase = 'finalizing'
          this.activeProviderRun = null
          this.currentAbortController = null
          this.activeTurnId = null
          this.activeTurnPhase = null
          this.steerQueue.clearCanceled()
          this.abortRecorded = false
          this.setPhase('idle')
          this.yields.arm()
        }
      }
    } finally {
      this.workerRunning = false
      this.resolveIdleWaiters()
      if (this.pendingActions.length > 0) this.kickWorker()
    }
  }

  private async executeAction(action: PendingAction): Promise<void> {
    switch (action.type) {
      case 'send':
        this.setPhase('running')
        await this.executeSend(action.content, action.hidden ?? false)
        return
      case 'retry':
        this.setPhase('running')
        await this.executeRetry()
        return
      case 'resume':
        this.setPhase('running')
        await this.executeResume()
        return
      case 'compact':
        this.setPhase('compacting')
        this.activeTurnPhase = 'compacting'
        await this.compaction.run()
        return
    }
  }

  /**
   * Applies a queued model/provider switch at a turn boundary (preflight). If the new model's
   * context window can't hold the current history, compaction runs FIRST with the current
   * (pre-switch) model + provider — which can still load it to summarize — and only then do we
   * swap. Doing it the other way would ask the smaller model to summarize a history it may not
   * be able to load.
   */
  private async applyPendingModelSwitch(): Promise<void> {
    const pending = this.pendingModelSwitch
    if (!pending) return
    this.pendingModelSwitch = null

    await this.compaction.compactToFit(pending.model)
    if (pending.provider && pending.provider !== this.provider) {
      const previous = this.provider
      this.provider = pending.provider
      await previous.dispose?.()
    }
    this.model = pending.model
  }

  private async runWithCompactingPhase<T>(fn: () => Promise<T>): Promise<T> {
    const previousPhase = this.currentPhase
    const previousActivePhase = this.activeTurnPhase
    this.setPhase('compacting')
    this.activeTurnPhase = 'compacting'
    try {
      return await fn()
    } finally {
      this.activeTurnPhase = previousActivePhase
      this.setPhase(previousPhase)
    }
  }

  private async executeSend(content: UserContentBlock[], hidden = false): Promise<void> {
    await this.runtime.lifecycle?.({
      type: 'before_round_start',
      agentSessionId: this.agentSessionId,
      state: this.agentState,
      transcript: this.transcriptLog,
      content,
    })

    const resolvedContent = await this.resolveReferences(content)
    await this.applyPendingModelSwitch()
    // Hidden internal turns (yield wakeups) are not user rounds, so they don't carry the preamble.
    const preamble = hidden ? null : (this.runtime.preamble?.(this.promptContext()) ?? null)
    this.transcriptLog.pushUserTurn(this.currentTurnId(), this.model, resolvedContent, preamble, hidden)
    await this.commitTranscript()

    await this.compaction.preflight()
    await this.turnLoop.run()
  }

  private async resolveReferences(content: UserContentBlock[]): Promise<UserContentBlock[]> {
    const resolver = this.runtime.resolveReferences
    if (!resolver) return content
    const signal = this.currentSignal()
    const resolved = await abortable(
      Promise.resolve(
        resolver(
          {
            state: this.agentState,
            agentSessionId: this.agentSessionId,
            cwd: this.cwd,
            transcript: this.transcriptLog,
            signal,
          },
          content,
        ),
      ),
      signal,
    )
    return resolved
  }

  private async executeRetry(): Promise<void> {
    const userBlock = this.transcriptLog.rewindToLastUserTurn()
    if (!userBlock) throw new Error('AgentSession: cannot retry without a user turn')
    this.activeTurnId = userBlock.turnId
    await this.runtime.lifecycle?.({
      type: 'after_transcript_rewrite',
      agentSessionId: this.agentSessionId,
      state: this.agentState,
      transcript: this.transcriptLog,
      reason: 'retry',
    })
    await this.commitTranscript()

    await this.applyPendingModelSwitch()
    await this.compaction.preflight()
    await this.turnLoop.run()
  }

  private async executeResume(): Promise<void> {
    await this.applyPendingModelSwitch()
    this.transcriptLog.markLatestAbortResumed()
    this.transcriptLog.pushResumeTurn(this.currentTurnId(), this.model)
    await this.commitTranscript()
    await this.compaction.preflight()
    await this.turnLoop.run()
  }

  private async *providerEvents(request: InferenceRequest, run: ProviderRun): AsyncIterable<ProviderEvent> {
    const iterator = run[Symbol.asyncIterator]()
    let completed = false
    try {
      while (true) {
        const next = await readProviderIterator(iterator, request.cancel)
        if (next.done) {
          completed = true
          return
        }
        yield next.value
      }
    } finally {
      if (!completed) void iterator.return?.().catch(noop)
    }
  }

  private promptContext(): {
    agentSessionId: string
    state: State
    cwd: string
    transcript: Transcript
  } {
    return {
      agentSessionId: this.agentSessionId,
      state: this.agentState,
      cwd: this.cwd,
      transcript: this.transcriptLog,
    }
  }

  private currentSignal(): AbortSignal {
    if (!this.currentAbortController) throw new Error('AgentSession: no active abort controller')
    return this.currentAbortController.signal
  }

  private currentTurnId(): string {
    if (!this.activeTurnId) throw new Error('AgentSession: no active turn id')
    return this.activeTurnId
  }

  private async recordAbort(): Promise<void> {
    if (this.abortRecorded) return
    this.abortRecorded = true
    await this.materializePendingSteersForCurrentTurn()
    for (const toolCall of this.transcriptLog.pendingToolCalls()) {
      this.transcriptLog.completeToolCall(
        toolCall.toolUseId,
        [{ type: 'text', text: `Tool call aborted: ${toolCall.toolName}` }],
        true,
      )
    }
    this.transcriptLog.pushAbort(this.model)
    await this.commitTranscript()
  }

  private async materializePendingSteersForCurrentTurn(): Promise<boolean> {
    if (!this.activeTurnId) return false
    const steers = this.steerQueue.takeForTurn(this.activeTurnId)
    if (steers.length === 0) return false
    for (const steer of steers) {
      this.transcriptLog.pushSteer(steer.turnId, steer.model, steer.content, steer.id, steer.hidden ?? false)
    }
    await this.commitTranscript()
    return true
  }

  private async materializePendingSteersArrivedSince(continuationCount: number): Promise<boolean> {
    if (this.steerQueue.continuationCount <= continuationCount) return false
    return this.materializePendingSteersForCurrentTurn()
  }

  private discardPendingSteersForCurrentTurn(): void {
    if (this.activeTurnId) this.steerQueue.takeForTurn(this.activeTurnId)
  }

  private removeQueuedMessage(id: string): void {
    const index = this.queued.findIndex((message) => message.id === id)
    if (index === -1) return
    this.queued.splice(index, 1)
    this.emitQueue()
  }

  private takeQueuedSend(id: string): TakenQueuedSend | null {
    const messageIndex = this.queued.findIndex((message) => message.id === id)
    if (messageIndex === -1) return null

    const [message] = this.queued.splice(messageIndex, 1)
    if (!message) return null

    const actionIndex = this.pendingActions.findIndex((action) => action.type === 'send' && action.id === id)
    const [action] = actionIndex === -1 ? [] : this.pendingActions.splice(actionIndex, 1)
    this.emitQueue()
    return {
      message,
      messageIndex,
      action: action && action.type === 'send' ? action : null,
      actionIndex,
    }
  }

  private restoreQueuedSend(queued: TakenQueuedSend): void {
    this.queued.splice(Math.min(queued.messageIndex, this.queued.length), 0, queued.message)
    if (queued.action) {
      const actionIndex = queued.actionIndex === -1 ? this.pendingActions.length : queued.actionIndex
      this.pendingActions.splice(Math.min(actionIndex, this.pendingActions.length), 0, queued.action)
    }
    this.emitQueue()
    this.kickWorker()
  }

  private removePendingSend(id: string): Extract<PendingAction, { type: 'send' }> | null {
    const actionIndex = this.pendingActions.findIndex((action) => action.type === 'send' && action.id === id)
    if (actionIndex === -1) return null
    const [action] = this.pendingActions.splice(actionIndex, 1)
    return action && action.type === 'send' ? action : null
  }

  private setPhase(phase: SessionPhase): void {
    if (this.currentPhase === phase) return
    this.currentPhase = phase
    this.emit({ type: 'phase_changed', phase })
  }

  private emitQueue(): void {
    this.emit({ type: 'queue_changed', queue: this.queuedMessages() })
  }

  /**
   * Publishes transcript changes: drains the mutation journal into a patch
   * event (O(changed content), not O(transcript)) and schedules a throttled
   * snapshot write. Boundaries (action end, abort, dispose) flush the write.
   */
  private async commitTranscript(): Promise<void> {
    const drained = this.transcriptLog.takePatches()
    if (drained) this.emit({ type: 'transcript_changed', patches: drained.patches, revision: drained.revision })
    this.schedulePersist()
  }

  private schedulePersist(): void {
    if (!this.store) return
    this.persistDirty = true
    if (this.persistTimer) return
    this.persistTimer = setTimeout(() => {
      this.persistTimer = null
      void this.persistSnapshot().catch((error: unknown) => {
        this.emit({ type: 'error', error: asError(error) })
      })
    }, this.persistIntervalMs)
  }

  private async persistSnapshot(): Promise<void> {
    if (!this.store || !this.persistDirty) return
    this.persistDirty = false
    await this.store.saveSnapshot({
      transcript: this.transcriptLog.snapshot(),
      state: structuredClone(this.agentState),
      phase: this.currentPhase,
      queue: structuredClone(this.queuedMessages()),
      cwd: this.cwd,
      model: structuredClone(this.model),
      harnessName: this.runtime.harnessName,
    })
  }

  private async flushPersist(): Promise<void> {
    if (this.persistTimer) {
      clearTimeout(this.persistTimer)
      this.persistTimer = null
    }
    await this.persistSnapshot()
  }

  private emit(event: SessionEvent): void {
    for (const listener of this.listeners) listener(event)
  }

  private resolveIdleWaiters(): void {
    const waiters = this.idleResolvers
    this.idleResolvers = []
    for (const resolve of waiters) resolve()
  }
}

function textContentSummary(content: UserContentBlock[]): string {
  const text = content
    .map((block) => (block.type === 'text' ? block.text : `[${block.type}]`))
    .join('\n')
    .trim()
  return truncate(text, 120, '...')
}

async function readProviderIterator(
  iterator: AsyncIterator<ProviderEvent>,
  signal: AbortSignal,
): Promise<IteratorResult<ProviderEvent>> {
  try {
    return await abortable(iterator.next(), signal)
  } catch (error) {
    if (isAbortError(error)) throw error
    const normalized = asError(error)
    return {
      done: false,
      value: {
        type: 'error',
        message: normalized.message,
        code: providerErrorCode(error),
      },
    }
  }
}

function providerErrorCode(error: unknown): string | null {
  if (error !== null && typeof error === 'object' && 'code' in error) {
    const code = (error as { code?: unknown }).code
    if (typeof code === 'string') return code
  }
  return null
}

