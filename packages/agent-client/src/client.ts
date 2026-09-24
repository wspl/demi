import {
  serverFrameSchema,
  type AbortResult,
  type Block,
  type ClientContent,
  type ClientFrame,
  type EditRequest,
  type ModelSelection,
  type ModelSwitchApply,
  type PendingSteer,
  type ProviderErrorDiagnostics,
  type QueuedMessage,
  type ServerFrame,
  type SessionPhase,
  type TranscriptVersion,
} from '@demicodes/protocol'
import { asError, createId } from '@demicodes/utils'
import type { AgentClientListener, ClientSessionEvent, Failures, ServerFrameOf } from './events'
import { applyTranscriptPatches } from './patch'
import type { AgentClientTransport } from './transport'

/** A correlated refusal: the edit was not accepted. */
export class EditRejectedError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'EditRejectedError'
  }
}

/** An `error` frame: a failed turn, a refused frame or a failed save, with its code and diagnostics. */
export class SessionError extends Error {
  readonly code: string | null
  readonly diagnostics: ProviderErrorDiagnostics | undefined

  constructor(frame: ServerFrameOf<'error'>) {
    super(frame.message)
    this.name = 'SessionError'
    this.code = frame.code ?? null
    this.diagnostics = frame.diagnostics
  }
}

/** How long a confirmation may take before its wait gives up. */
const CONFIRMATION_MS = 30_000

type ActionCommand = 'send' | 'retry' | 'resume' | 'compact'

/** A wait for the action a frame starts, which settles when its phase cycle ends. */
interface ActionWaiter {
  command: ActionCommand
  messageId: string | null
  /** Whether the session has left `idle` for this action. */
  started: boolean
  resolve: () => void
  reject: (error: Error) => void
}

interface Waiter<T> {
  resolve: (value: T) => void
  reject: (error: Error) => void
}

/**
 * Where the client's copy of the root transcript is: none yet, current at a
 * version, or stale after a gap in the patch revisions, until the reset the
 * client asked for arrives.
 */
type TranscriptState =
  | { status: 'none' }
  | { status: 'current'; version: TranscriptVersion }
  | { status: 'stale' }

/** What a wait decides about one event: to settle, or to keep waiting. */
type Decision<T> = { resolve: T } | { reject: Error } | undefined

/**
 * The browser's client of the conversation socket's frame protocol
 * (`runtime.md` § Frame protocol). It validates every frame it receives with
 * the generated schemas and drops the connection when one does not match,
 * applies patches with the one patch applier, and keeps the transcript, the
 * phase, the queue and the pending steers.
 */
export class AgentClient {
  private readonly transport: AgentClientTransport
  private readonly listeners = new Set<AgentClientListener>()
  private readonly actionWaiters: ActionWaiter[] = []
  private readonly steerWaiters = new Map<string, Waiter<void>>()
  private readonly abortWaiters: Waiter<AbortResult>[] = []
  private readonly detachTransport: () => void
  private disconnected = false
  private transcriptState: TranscriptState = { status: 'none' }
  private blocks: Block[] = []
  /** The facts of the root's error blocks, beside the blocks, never inside them. */
  private failures: Failures = {}
  /** Each subagent transcript's revision, for the same patch rule as the root's. */
  private readonly subagentRevisions = new Map<string, number>()
  private phase: SessionPhase | null = null
  private queue: QueuedMessage[] = []
  private pending: PendingSteer[] = []

  constructor(transport: AgentClientTransport) {
    this.transport = transport
    const stopFrames = transport.onFrame((frame) => this.receive(frame))
    const stopClose = transport.onClose((error) => this.disconnect(error))
    this.detachTransport = () => {
      stopFrames()
      stopClose()
    }
  }

  /** Attaches to the conversation's tree and selects `model`; resolves on `opened`. */
  open(model: ModelSelection): Promise<void> {
    return this.request({ type: 'open', model }, (event) => {
      if (event.type === 'opened') {
        return { resolve: undefined }
      }
      return this.failure(event, 'open')
    })
  }

  /** Sends a message; resolves when the action it starts ends. */
  send(content: ClientContent[]): Promise<void> {
    const messageId = createId()
    return this.action('send', messageId, { type: 'send', messageId, content })
  }

  /** Sends a message; resolves once it is in the transcript or the queue. */
  submit(content: ClientContent[], messageId: string = createId()): Promise<void> {
    return this.request(
      { type: 'send', messageId, content },
      (event) => {
        const written = (event.type === 'transcript_reset' || event.type === 'transcript_patch')
          && event.blocks.some((block) => block.type === 'user' && block.turnId === messageId)
        const queued = event.type === 'queue' && event.queue.some((message) => message.id === messageId)
        if (written || queued) {
          return { resolve: undefined }
        }
        return this.failure(event, 'send')
      },
      'Message confirmation timed out',
    )
  }

  /** Resolves on the edit's own durable receipt, whatever the transcript shows before it. */
  editAndSend(request: EditRequest): Promise<void> {
    return this.request(
      { type: 'edit_and_send', request },
      (event) => {
        if (event.type === 'edit_result' && event.operationId === request.operationId) {
          return event.outcome.status === 'accepted'
            ? { resolve: undefined }
            : { reject: new EditRejectedError(event.outcome.reason) }
        }
        if (event.type === 'rejected' && event.command === 'edit_and_send') {
          return { reject: new EditRejectedError(event.reason) }
        }
        if (event.type === 'error' && event.code === 'invalid_frame') {
          return { reject: new SessionError(event) }
        }
        if (event.type === 'closed' || event.type === 'disconnected') {
          return { reject: new Error('Agent connection closed before the edit was confirmed') }
        }
        return undefined
      },
      'Edit confirmation timed out; retry to check acceptance',
    )
  }

  dequeueMessage(messageId: string): void {
    if (!this.sendFrame({ type: 'dequeue_message', messageId })) {
      return
    }
    this.settleQueuedSend(messageId)
  }

  /** Moves a queued message to the front, so that it runs next. */
  sendQueuedMessage(messageId: string): void {
    if (!this.sendFrame({ type: 'send_queued_message', messageId })) {
      return
    }
    const waiter = this.queuedSendWaiter(messageId)
    if (!waiter) {
      return
    }
    this.actionWaiters.splice(this.actionWaiters.indexOf(waiter), 1)
    const next = this.actionWaiters.findIndex((candidate) => candidate.command === 'send' && !candidate.started)
    this.actionWaiters.splice(next === -1 ? this.actionWaiters.length : next, 0, waiter)
  }

  /** Turns a queued message into a steer of the running turn; resolves when the steer is accepted. */
  async steerQueuedMessage(messageId: string, steerId: string = createId()): Promise<void> {
    await this.steerRequest(steerId, { type: 'steer_queued_message', messageId, steerId })
    this.settleQueuedSend(messageId)
  }

  clearMessageQueue(): void {
    if (!this.sendFrame({ type: 'clear_message_queue' })) {
      return
    }
    for (const message of this.queue) {
      this.settleQueuedSend(message.id)
    }
  }

  /** Adds input to the running turn; resolves when the steer is accepted. */
  steer(content: ClientContent[], steerId: string = createId()): Promise<void> {
    return this.steerRequest(steerId, { type: 'steer', steerId, content })
  }

  cancelPendingSteer(steerId: string): void {
    this.sendFrame({ type: 'cancel_pending_steer', steerId })
  }

  /**
   * Changes the model selection. `next_turn`, the default, applies it at the
   * next action; `immediate` at the next continuation boundary of the
   * running turn (`runtime.md` § Model switch).
   */
  setProvider(model: ModelSelection, apply?: ModelSwitchApply): void {
    this.sendFrame({ type: 'set_provider', model, apply })
  }

  /** Runs the latest turn again from the user's input; resolves when it ends. */
  retry(): Promise<void> {
    return this.action('retry', null, { type: 'retry' })
  }

  /** Finishes a turn that did not finish; resolves when it ends. */
  resume(): Promise<void> {
    return this.action('resume', null, { type: 'resume' })
  }

  compact(): Promise<void> {
    return this.action('compact', null, { type: 'compact' })
  }

  /** Stops one thing; answers what it stopped, in request order. */
  abort(): Promise<AbortResult> {
    if (this.disconnected) {
      return Promise.reject(new Error('Agent connection closed'))
    }
    return new Promise((resolve, reject) => {
      this.abortWaiters.push({ resolve, reject })
      this.sendFrame({ type: 'abort' })
    })
  }

  /** Stops every live subagent; each settles through its own `subagent` frame. */
  abortSubagents(): void {
    this.sendFrame({ type: 'abort_subagents' })
  }

  /** Stops one live subagent with its subtree. */
  abortSubagent(subagentId: string): void {
    this.sendFrame({ type: 'abort_subagent', subagentId })
  }

  /** Stops a running command; its final status arrives as `shell_output`. */
  shellAbort(commandId: string): void {
    this.sendFrame({ type: 'shell_abort', commandId })
  }

  /** Writes stdin to a running command; resolves on its `shell_write_result`. */
  shellWrite(commandId: string, stdin: string): Promise<void> {
    return this.request({ type: 'shell_write', commandId, stdin }, (event) => {
      if (event.type === 'shell_write_result' && event.commandId === commandId) {
        return { resolve: undefined }
      }
      return this.failure(event, 'shell_write')
    })
  }

  /** Disposes the conversation's tree, then detaches. */
  async close(): Promise<void> {
    try {
      await this.request({ type: 'close' }, (event) => {
        if (event.type === 'closed') {
          return { resolve: undefined }
        }
        return event.type === 'disconnected' ? { reject: event.error } : undefined
      })
    } finally {
      this.disconnect()
    }
  }

  /** Detaches without the frame that disposes the tree, which keeps running. */
  disconnect(error: Error = new Error('Agent connection closed')): void {
    if (this.disconnected) {
      return
    }
    this.disconnected = true
    this.detachTransport()
    this.transport.close()
    this.rejectWaiters(error)
    this.emit({ type: 'disconnected', error })
    this.listeners.clear()
  }

  subscribe(listener: AgentClientListener): () => void {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  transcript(): { blocks: Block[] } {
    return { blocks: [...this.blocks] }
  }

  /** The version of the transcript the client holds, or null while it holds none it can vouch for. */
  transcriptVersion(): TranscriptVersion | null {
    if (this.disconnected || this.transcriptState.status !== 'current') {
      return null
    }
    return { ...this.transcriptState.version }
  }

  /** The accepted steers not yet in the transcript, as copies. */
  pendingSteers(): PendingSteer[] {
    return structuredClone(this.pending)
  }

  /** Sends `frame` unless the client is detached; answers whether it did. */
  private sendFrame(frame: ClientFrame): boolean {
    if (this.disconnected) {
      return false
    }
    try {
      this.transport.send(frame)
    } catch (error) {
      this.disconnect(asError(error))
      return false
    }
    return true
  }

  /**
   * Sends `frame` and waits for the event `decide` settles on; with a
   * `timeoutMessage`, the wait fails after `CONFIRMATION_MS`.
   */
  private request<T>(
    frame: ClientFrame,
    decide: (event: ClientSessionEvent) => Decision<T>,
    timeoutMessage?: string,
  ): Promise<T> {
    if (this.disconnected) {
      return Promise.reject(new Error('Agent connection closed'))
    }
    return new Promise((resolve, reject) => {
      const finish = (decision: NonNullable<Decision<T>>) => {
        clearTimeout(timeout)
        unsubscribe()
        if ('resolve' in decision) {
          resolve(decision.resolve)
        } else {
          reject(decision.reject)
        }
      }
      const unsubscribe = this.subscribe((event) => {
        const decision = decide(event)
        if (decision) {
          finish(decision)
        }
      })
      const timeout = timeoutMessage === undefined
        ? undefined
        : setTimeout(() => finish({ reject: new Error(timeoutMessage) }), CONFIRMATION_MS)
      this.sendFrame(frame)
    })
  }

  /** The events that end a wait for a reply to `command` with a failure. */
  private failure(event: ClientSessionEvent, command: ClientFrame['type']): Decision<never> {
    switch (event.type) {
      case 'error':
        return { reject: new SessionError(event) }
      case 'rejected':
        return event.command === command ? { reject: new Error(event.reason) } : undefined
      case 'closed':
        return { reject: new Error('Agent connection closed') }
      case 'disconnected':
        return { reject: event.error }
      default:
        return undefined
    }
  }

  private action(command: ActionCommand, messageId: string | null, frame: ClientFrame): Promise<void> {
    if (this.disconnected) {
      return Promise.reject(new Error('Agent connection closed'))
    }
    return new Promise((resolve, reject) => {
      this.actionWaiters.push({ command, messageId, started: false, resolve, reject })
      this.sendFrame(frame)
    })
  }

  private steerRequest(steerId: string, frame: ClientFrame): Promise<void> {
    if (this.disconnected) {
      return Promise.reject(new Error('Agent connection closed'))
    }
    return new Promise((resolve, reject) => {
      this.steerWaiters.set(steerId, { resolve, reject })
      this.sendFrame(frame)
    })
  }

  /** The wait of a queued `send` that has not started. */
  private queuedSendWaiter(messageId: string): ActionWaiter | undefined {
    return this.actionWaiters.find(
      (waiter) => waiter.command === 'send' && waiter.messageId === messageId && !waiter.started,
    )
  }

  /** A queued message left the queue without running: its send is done. */
  private settleQueuedSend(messageId: string): void {
    const waiter = this.queuedSendWaiter(messageId)
    if (waiter) {
      this.settleAction(waiter, () => waiter.resolve())
    }
  }

  private settleAction(waiter: ActionWaiter, settle: () => void): void {
    const index = this.actionWaiters.indexOf(waiter)
    if (index === -1) {
      return
    }
    this.actionWaiters.splice(index, 1)
    settle()
  }

  private rejectWaiters(error: Error): void {
    for (const waiter of this.actionWaiters.splice(0)) {
      waiter.reject(error)
    }
    this.rejectSteersAndAborts(error)
  }

  private rejectSteersAndAborts(error: Error): void {
    const steers = [...this.steerWaiters.values()]
    this.steerWaiters.clear()
    for (const waiter of steers) {
      waiter.reject(error)
    }
    for (const waiter of this.abortWaiters.splice(0)) {
      waiter.reject(error)
    }
  }

  /**
   * The trust boundary: a frame is validated against the generated schema
   * before anything acts on it. A frame that does not match cannot be
   * understood in part, so the connection drops with the reason.
   */
  private receive(frame: unknown): void {
    const parsed = serverFrameSchema.safeParse(frame)
    if (!parsed.success) {
      const issue = parsed.error.issues[0]
      const where = issue ? `: ${issue.path.join('.')} ${issue.message}` : ''
      this.disconnect(new Error(`Invalid server frame${where}`))
      return
    }
    this.handle(parsed.data)
  }

  private handle(frame: ServerFrame): void {
    switch (frame.type) {
      case 'opened':
        this.pending = []
        this.emit(frame)
        return
      case 'transcript_reset':
        this.blocks = [...frame.blocks]
        this.failures = { ...frame.failures }
        this.transcriptState = { status: 'current', version: frame.version }
        this.removeWrittenSteers(true)
        this.emit({ type: 'transcript_reset', blocks: this.blocks, failures: this.failures })
        return
      case 'transcript_patch':
        this.patchTranscript(frame)
        return
      case 'phase': {
        const previous = this.phase
        this.phase = frame.phase
        this.emit(frame)
        this.followPhase(previous, frame.phase)
        return
      }
      case 'queue':
        this.queue = frame.queue
        this.emit(frame)
        return
      case 'pending_steers':
        this.pending = structuredClone(frame.pendingSteers)
        this.removeWrittenSteers(false)
        this.emitPendingSteers()
        return
      case 'steer_result':
        this.emit(frame)
        this.settleSteer(frame)
        return
      case 'abort_result':
        this.emit(frame)
        this.abortWaiters.shift()?.resolve(frame.result)
        return
      case 'subagent_transcript_reset':
        this.subagentRevisions.set(frame.subagentId, frame.revision)
        this.emit({
          type: 'subagent_transcript_reset',
          subagentId: frame.subagentId,
          blocks: frame.blocks,
          failures: frame.failures ?? {},
        })
        return
      case 'subagent_transcript_patch':
        this.patchSubagentTranscript(frame)
        return
      case 'rejected':
        this.emit(frame)
        this.rejectAction(frame.command, new Error(frame.reason))
        if (frame.command === 'abort') {
          for (const waiter of this.abortWaiters.splice(0)) {
            waiter.reject(new Error(frame.reason))
          }
        }
        return
      case 'error': {
        this.emit(frame)
        // A turn that failed is a transcript record; the wait learns it by the error's type.
        const error = new SessionError(frame)
        const active = this.actionWaiters.find((waiter) => waiter.started)
        if (active) {
          this.settleAction(active, () => active.reject(error))
        } else {
          for (const waiter of this.actionWaiters.splice(0)) {
            waiter.reject(error)
          }
        }
        this.rejectSteersAndAborts(error)
        return
      }
      case 'closed':
        this.transcriptState = { status: 'none' }
        this.blocks = []
        this.failures = {}
        this.subagentRevisions.clear()
        this.phase = null
        this.queue = []
        this.pending = []
        this.emit(frame)
        for (const waiter of this.actionWaiters.splice(0)) {
          waiter.resolve()
        }
        this.rejectSteersAndAborts(new Error('Session closed'))
        return
      case 'edit_result':
      case 'shell_output':
      case 'shell_write_result':
      case 'retry_scheduled':
      case 'subagent':
        this.emit(frame)
        return
    }
  }

  /**
   * Applies a patch whose revision is one past the client's, ignores one that
   * is not past it, and asks for a fresh transcript at a gap
   * (`runtime.md` § Order and delivery).
   */
  private patchTranscript(frame: ServerFrameOf<'transcript_patch'>): void {
    const state = this.transcriptState
    if (state.status === 'stale') {
      return
    }
    if (state.status === 'current' && frame.revision <= state.version.revision) {
      return
    }
    if (state.status === 'none' || frame.revision !== state.version.revision + 1) {
      this.resync()
      return
    }
    this.transcriptState = { status: 'current', version: { epoch: state.version.epoch, revision: frame.revision } }
    this.blocks = applyTranscriptPatches(this.blocks, frame.patches)
    this.failures = { ...this.failures, ...frame.failures }
    this.removeWrittenSteers(true)
    this.emit({ type: 'transcript_patch', patches: frame.patches, blocks: this.blocks, failures: this.failures })
  }

  /** The root's rule, for each subagent's stream. */
  private patchSubagentTranscript(frame: ServerFrameOf<'subagent_transcript_patch'>): void {
    if (this.transcriptState.status === 'stale') {
      return
    }
    const revision = this.subagentRevisions.get(frame.subagentId)
    if (revision !== undefined && frame.revision <= revision) {
      return
    }
    if (revision === undefined || frame.revision !== revision + 1) {
      this.resync()
      return
    }
    this.subagentRevisions.set(frame.subagentId, frame.revision)
    this.emit({
      type: 'subagent_transcript_patch',
      subagentId: frame.subagentId,
      patches: frame.patches,
      failures: frame.failures ?? {},
    })
  }

  /** A patch was missed: the client asks for the transcripts again and ignores patches until they arrive. */
  private resync(): void {
    this.transcriptState = { status: 'stale' }
    this.sendFrame({ type: 'sync_transcript' })
  }

  /** Drops the pending steers the transcript now holds, by id. */
  private removeWrittenSteers(announce: boolean): void {
    if (this.pending.length === 0) {
      return
    }
    const written = new Set(this.blocks.filter((block) => block.type === 'steer').map((block) => block.id))
    const remaining = this.pending.filter((steer) => !written.has(steer.id))
    if (remaining.length === this.pending.length) {
      return
    }
    this.pending = remaining
    if (announce) {
      this.emitPendingSteers()
    }
  }

  private emitPendingSteers(): void {
    this.emit({ type: 'pending_steers', pendingSteers: this.pendingSteers() })
  }

  private settleSteer(frame: ServerFrameOf<'steer_result'>): void {
    const waiter = this.steerWaiters.get(frame.steerId)
    if (!waiter) {
      return
    }
    this.steerWaiters.delete(frame.steerId)
    if (frame.outcome.status === 'accepted') {
      waiter.resolve()
    } else {
      waiter.reject(new Error(frame.outcome.reason))
    }
  }

  /** An action starts when the session leaves `idle` and ends when it returns. */
  private followPhase(previous: SessionPhase | null, phase: SessionPhase): void {
    if (phase !== 'idle') {
      if (previous === 'idle' || previous === null) {
        const next = this.actionWaiters.find((waiter) => !waiter.started)
        if (next) {
          next.started = true
        }
      }
      return
    }
    const active = this.actionWaiters.find((waiter) => waiter.started)
    if (active) {
      this.settleAction(active, () => active.resolve())
    }
  }

  /** A refused command fails the first wait for it that has not started, else the first. */
  private rejectAction(command: string, error: Error): void {
    const waiter = this.actionWaiters.find((candidate) => candidate.command === command && !candidate.started)
      ?? this.actionWaiters.find((candidate) => candidate.command === command)
    if (waiter) {
      this.settleAction(waiter, () => waiter.reject(error))
    }
  }

  private emit(event: ClientSessionEvent): void {
    for (const listener of [...this.listeners]) {
      listener(event)
    }
  }
}
