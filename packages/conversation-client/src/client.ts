import {
  serverFrameSchema,
  type AbortResult,
  type Block,
  type ClientContent,
  type ClientFrame,
  type EditRequest,
  type MessageComment,
  type PendingCall,
  type PendingSteer,
  type ProviderErrorDiagnostics,
  type QueuedMessage,
  type RegenerateRequest,
  type RemoveOutcome,
  type SavedDelivery,
  type ServerFrame,
  type SessionPhase,
  type TranscriptVersion,
  schemaRefusal,
} from '@demicodes/protocol'
import { MAX_PAGE_MESSAGE_BYTES } from '@demicodes/protocol'
import { asError, createId } from '@demicodes/utils'
import type { ConversationClientListener, ClientSessionEvent, ServerFrameOf } from './events'
import { FrameTooLargeError, type ConversationClientTransport } from './transport'

/** The root's blocks the page holds, as `open` and `sync_transcript` name them (`rendering.md` § Where a reset starts). */
export interface HeldEdge {
  from: number
  edge: string
}

/**
 * A frame the backend or the session refused: the code the page words it
 * by and the values that text needs, with the refusal in English as the
 * message, and the request ID of the socket that refused it, which a `rejected`
 * frame carries.
 */
export class RefusedError extends Error {
  override name = 'RefusedError'
  readonly code: string
  readonly params: Readonly<Record<string, unknown>>
  /** The request ID of the socket that sent the refusal, which the page shows with it; null when the frame names none. */
  readonly requestId: string | null

  constructor(refusal: { code: string; params?: Record<string, unknown>; reason: string; requestId?: string }) {
    super(refusal.reason)
    this.code = refusal.code
    this.params = refusal.params ?? {}
    this.requestId = refusal.requestId ?? null
  }
}

/** A correlated refusal: the edit was not accepted. */
export class EditRejectedError extends RefusedError {
  override name = 'EditRejectedError'
}

/** A correlated refusal: the steer was not accepted, and a queued message it came from stays queued. */
export class SteerRejectedError extends RefusedError {
  override name = 'SteerRejectedError'
}

/** A correlated refusal: the message saved for later is no longer on the list, as another page sent or removed it. */
export class SavedGoneError extends Error {
  constructor() {
    super('The message is no longer saved for later')
    this.name = 'SavedGoneError'
  }
}

/**
 * The connection ended while the session was running the action: it had
 * taken it, so the action was accepted, and only the wait for its end is
 * lost. What became of it is in the snapshot of the next connection, not a
 * refusal.
 */
export class ActionOutlivedError extends Error {
  constructor(cause: Error) {
    super('The connection ended while the session ran the action', { cause })
    this.name = 'ActionOutlivedError'
  }
}

/** An `error` frame: a failed turn, a refused frame or a failed save, with its code and diagnostics. */
export class SessionError extends Error {
  readonly code: string | null
  readonly diagnostics: ProviderErrorDiagnostics | undefined
  /** The request ID of the socket that sent the frame, which the page shows with the error. */
  readonly requestId: string | null

  constructor(frame: ServerFrameOf<'error'>) {
    super(frame.message)
    this.name = 'SessionError'
    this.code = frame.code ?? null
    this.diagnostics = frame.diagnostics
    this.requestId = frame.requestId ?? null
  }
}

/** How long a confirmation may take before its wait gives up. */
const CONFIRMATION_MS = 30_000

type ActionCommand = 'send' | 'retry' | 'resume' | 'compact'

/**
 * A wait for the action a frame starts. `sent` until the session takes the
 * action: a send is `queued` while it waits behind another action, and
 * `running` once its message is written; any other action is `running` once
 * the session leaves `idle` after it was sent.
 */
interface ActionWaiter {
  command: ActionCommand
  messageId: string | null
  /** The subagent whose action it is, whose phase it follows; null for the root's. */
  node: string | null
  status: 'sent' | 'queued' | 'running'
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
 * The web app's client of the conversation socket's frame protocol
 * (`rendering.md` § Frame protocol). It validates every frame it receives with
 * the generated schemas and drops the connection when one does not match,
 * keeps the patch revisions, the phase, the queue and the pending steers,
 * and tells its listeners what the transcript frames bring, which the page
 * applies to the parts of the transcript it holds with the one patch
 * applier. Of the transcript it keeps only which messages and steers it has
 * seen written, which its waits settle on.
 */
export class ConversationClient {
  private readonly transport: ConversationClientTransport
  private readonly listeners = new Set<ConversationClientListener>()
  private readonly actionWaiters: ActionWaiter[] = []
  private readonly steerWaiters = new Map<string, Waiter<void>>()
  private readonly abortWaiters: Waiter<AbortResult>[] = []
  private readonly detachTransport: () => void
  private disconnected = false
  private transcriptState: TranscriptState = { status: 'none' }
  /** The turns of the `user` blocks the stream has brought since the last reset. */
  private readonly writtenTurns = new Set<string>()
  /** The ids of the `steer` blocks the stream has brought since the last reset. */
  private readonly writtenSteers = new Set<string>()
  /** The root's blocks the page holds, which `open` and `sync_transcript` name. */
  private held: () => HeldEdge | undefined = () => undefined
  /** The index the last `open` or `sync_transcript` named. */
  private asked: number | undefined
  /** Each subagent transcript's revision, for the same patch rule as the root's. */
  private readonly subagentRevisions = new Map<string, number>()
  /** The children whose transcripts the page shows, as it last named them (`crates/agent-server/src/server/tree.rs`). */
  private shown = new Set<string>()
  /**
   * The children the page stopped showing: a frame of their transcripts
   * still on its way is dropped, until the page shows them again and their
   * reset arrives. A side question's node, which no page names, is never one.
   */
  private readonly hidden = new Set<string>()
  /** Each live subagent's phase, which a compact of that child follows. */
  private readonly subagentPhases = new Map<string, SessionPhase>()
  private phase: SessionPhase | null = null
  private queue: QueuedMessage[] = []
  private pending: PendingSteer[] = []
  /** The calls the root's model is writing, as the server last listed them (`rendering.md` § Calls being written). */
  private calls: PendingCall[] = []

  constructor(transport: ConversationClientTransport) {
    this.transport = transport
    const stopFrames = transport.onFrame((frame) => this.receive(frame))
    const stopClose = transport.onClose((error) => this.disconnect(error))
    this.detachTransport = () => {
      stopFrames()
      stopClose()
    }
  }

  /**
   * Attaches to the conversation's tree, which runs with the model settings
   * the conversation's record holds; resolves on `opened`. `held` names the
   * root's blocks the page holds, now and at each later sync, so the stream
   * sends none before them (`rendering.md` § Where a reset starts).
   */
  open(held: () => HeldEdge | undefined = () => undefined): Promise<void> {
    this.held = held
    const edge = held()
    this.asked = edge?.from
    return this.request({ type: 'open', ...edge }, (event) => {
      if (event.type === 'opened') {
        return { resolve: undefined }
      }
      return this.failure(event, 'open')
    })
  }

  /** Sends a message; resolves when the action it starts ends, or once it leaves the queue without running. */
  send(content: ClientContent[]): Promise<void> {
    const messageId = createId()
    return this.action('send', messageId, { type: 'send', messageId, content })
  }

  /**
   * Sends a message, with the comments it carries; resolves once it is in
   * the transcript or the queue. The wait has no deadline of its own: the
   * backend decides that a delivery failed, and says why.
   */
  submit(content: ClientContent[], messageId: string = createId(), comments: readonly MessageComment[] = []): Promise<void> {
    return this.request(
      { type: 'send', messageId, content, ...(comments.length > 0 ? { comments: [...comments] } : {}) },
      (event) => {
        const written = (event.type === 'transcript_reset' || event.type === 'transcript_patch')
          && this.writtenTurns.has(messageId)
        const queued = event.type === 'queue' && event.queue.some((message) => message.id === messageId)
        if (written || queued) {
          return { resolve: undefined }
        }
        return this.failure(event, 'send')
      },
    )
  }

  /**
   * Sends the message saved for later `id` the way the composer chose, as the message
   * `turnId`, the page's id for this send; resolves once the session holds it: in the
   * transcript or the queue, or accepted as a steer, at once when it does already. A
   * steer the session refuses rejects with `SteerRejectedError`, a message the list no
   * longer holds, which no send took as `turnId`, with `SavedGoneError`, and any other
   * refusal of the backend's with `RefusedError`; the wait has no deadline of its own.
   */
  sendSaved(id: string, turnId: string, delivery: SavedDelivery): Promise<void> {
    if (this.writtenTurns.has(turnId) || this.queue.some((message) => message.id === turnId)) {
      return Promise.resolve()
    }
    return this.request(
      { type: 'send_saved', id, turnId, delivery },
      (event) => {
        if (event.type === 'steer_result' && event.steerId === turnId) {
          return event.outcome.status === 'accepted'
            ? { resolve: undefined }
            : { reject: new SteerRejectedError(event.outcome) }
        }
        const written = (event.type === 'transcript_reset' || event.type === 'transcript_patch')
          && this.writtenTurns.has(turnId)
        const queued = event.type === 'queue' && event.queue.some((message) => message.id === turnId)
        if (delivery !== 'steer' && (written || queued)) {
          return { resolve: undefined }
        }
        if (event.type === 'rejected' && event.command === 'send_saved' && event.code === 'saved_not_found') {
          return { reject: new SavedGoneError() }
        }
        return this.failure(event, 'send_saved')
      },
    )
  }

  /** Resolves on the edit's own durable receipt, whatever the transcript shows before it. */
  editAndSend(request: EditRequest): Promise<void> {
    return this.editRequest({ type: 'edit_and_send', request }, request.operationId)
  }

  /**
   * Regenerate (`message-editing.md` § Regenerate and the Up Arrow key): a
   * new version of the answer to the message `request` names; resolves on
   * its durable receipt, as an edit does.
   */
  regenerate(request: RegenerateRequest): Promise<void> {
    return this.editRequest({ type: 'regenerate', request }, request.operationId)
  }

  /**
   * Makes the `to`th version at the place of the block `blockId` current
   * (`message-editing.md` § Versions), from the transcript `version` the
   * page shows. The rewrite reaches the transcript as patches; a refusal
   * arrives as a `rejected` frame.
   */
  switchVersion(blockId: string, to: number, version: TranscriptVersion): void {
    this.sendFrame({ type: 'switch_version', blockId, to, version })
  }

  /** An edit's or a Regenerate's request, which its `edit_result` answers. */
  private editRequest(
    frame: Extract<ClientFrame, { type: 'edit_and_send' | 'regenerate' }>,
    operationId: string,
  ): Promise<void> {
    return this.request(
      frame,
      (event) => {
        if (event.type === 'edit_result' && event.operationId === operationId) {
          return event.outcome.status === 'accepted'
            ? { resolve: undefined }
            : { reject: new EditRejectedError(event.outcome) }
        }
        if (event.type === 'rejected' && event.command === frame.type) {
          return { reject: new EditRejectedError(event) }
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

  /**
   * Removes a queued message; resolves with the session's answer: `removed`,
   * or `gone_on` when its turn had started or the session never held it
   * (`runtime.md` § Pending steers).
   */
  async dequeueMessage(messageId: string): Promise<RemoveOutcome> {
    const outcome = await this.removeRequest(messageId, { type: 'dequeue_message', messageId })
    if (outcome === 'removed') {
      this.settleQueuedSend(messageId)
    }
    return outcome
  }

  /**
   * Sends a queued message now (`runtime.md` § Send now): while a turn runs,
   * the turn ends at once and the message runs next; otherwise it moves to
   * the front of the queue. The caller watches the queue and the transcript.
   */
  sendQueuedMessage(messageId: string): void {
    this.sendFrame({ type: 'send_queued_message', messageId })
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
  steer(content: ClientContent[], steerId: string = createId(), comments: readonly MessageComment[] = []): Promise<void> {
    return this.steerRequest(steerId, { type: 'steer', steerId, content, ...(comments.length > 0 ? { comments: [...comments] } : {}) })
  }

  /**
   * Delivers a pending steer now (`runtime.md` § Send now): the running call
   * returns at once with its command still running, and the turn goes on
   * with the steer. The caller watches the pending steers and the transcript.
   */
  steerNow(steerId: string): void {
    this.sendFrame({ type: 'steer_now', steerId })
  }

  /**
   * Withdraws a pending steer; resolves with the session's answer:
   * `removed`, or `gone_on` when the turn had taken it or the session never
   * held it (`runtime.md` § Pending steers).
   */
  cancelPendingSteer(steerId: string): Promise<RemoveOutcome> {
    return this.removeRequest(steerId, { type: 'cancel_pending_steer', steerId })
  }

  /**
   * Sends a removal and waits for its `remove_result`, which follows the
   * list's frame. A failed turn does not fail it: its `error` frame is the
   * turn's, not the removal's.
   */
  private removeRequest(id: string, frame: ClientFrame): Promise<RemoveOutcome> {
    return this.request(
      frame,
      (event) => {
        if (event.type === 'remove_result' && event.id === id) {
          return { resolve: event.outcome }
        }
        return event.type === 'error' ? undefined : this.failure(event, frame.type)
      },
      'The removal was not confirmed.',
    )
  }

  /** Runs the latest turn again from the user's input; resolves when it ends. */
  retry(): Promise<void> {
    return this.action('retry', null, { type: 'retry' })
  }

  /** Finishes a turn that did not finish; resolves when it ends. */
  resume(): Promise<void> {
    return this.action('resume', null, { type: 'resume' })
  }

  /**
   * Compacts the context of `node`, a live subagent, or the root's when
   * absent; resolves when that agent's compaction ends.
   */
  compact(node?: string): Promise<void> {
    return node === undefined
      ? this.action('compact', null, { type: 'compact' })
      : this.action('compact', null, { type: 'compact', node }, node)
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

  /**
   * Names the children whose transcripts the page shows, the whole set: a
   * live child it adds streams its transcript from its reset on, and one it
   * drops streams no more. Sent before `open`, it names those the open sends.
   */
  showSubagents(ids: readonly string[]): void {
    for (const id of this.shown) {
      if (!ids.includes(id)) {
        this.hidden.add(id)
        this.subagentRevisions.delete(id)
      }
    }
    for (const id of ids) {
      this.hidden.delete(id)
    }
    this.shown = new Set(ids)
    this.sendFrame({ type: 'show_subagents', ids: [...ids] })
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

  subscribe(listener: ConversationClientListener): () => void {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
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

  /** The calls the root's model is writing, as copies. */
  pendingCalls(): PendingCall[] {
    return structuredClone(this.calls)
  }

  /** Sends `frame` unless the client is detached; answers whether it did. */
  private sendFrame(frame: ClientFrame): boolean {
    if (this.disconnected) {
      return false
    }
    try {
      this.transport.send(frame)
    } catch (error) {
      if (error instanceof FrameTooLargeError) {
        this.refuse(frame, { code: 'too_large', params: { limitBytes: MAX_PAGE_MESSAGE_BYTES }, reason: error.message })
        return false
      }
      this.disconnect(asError(error))
      return false
    }
    return true
  }

  /** Answers a frame the transport would not send as the backend answers a frame it refuses. */
  private refuse(frame: ClientFrame, refusal: { code: string; params: Record<string, number>; reason: string }): void {
    if (frame.type === 'steer') {
      this.handle({ type: 'steer_result', steerId: frame.steerId, outcome: { status: 'rejected', ...refusal } })
      return
    }
    this.handle({ type: 'rejected', command: frame.type, ...refusal })
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
        return event.command === command ? { reject: new RefusedError(event) } : undefined
      case 'closed':
        return { reject: new Error('Agent connection closed') }
      case 'disconnected':
        return { reject: event.error }
      default:
        return undefined
    }
  }

  private action(command: ActionCommand, messageId: string | null, frame: ClientFrame, node: string | null = null): Promise<void> {
    if (this.disconnected) {
      return Promise.reject(new Error('Agent connection closed'))
    }
    return new Promise((resolve, reject) => {
      this.actionWaiters.push({ command, messageId, node, status: 'sent', resolve, reject })
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

  /** The wait of a `send` whose message has not run. */
  private queuedSendWaiter(messageId: string): ActionWaiter | undefined {
    return this.actionWaiters.find(
      (waiter) => waiter.command === 'send' && waiter.messageId === messageId && waiter.status !== 'running',
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
      waiter.reject(waiter.status === 'sent' ? error : new ActionOutlivedError(error))
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
      this.disconnect(new Error(`Invalid server frame:\n${schemaRefusal(parsed.error)}`))
      return
    }
    this.handle(parsed.data)
  }

  private handle(frame: ServerFrame): void {
    switch (frame.type) {
      case 'opened':
        this.pending = []
        this.calls = []
        this.emit(frame)
        return
      case 'transcript_reset':
        this.writtenTurns.clear()
        this.writtenSteers.clear()
        this.noteWritten(frame.blocks)
        this.transcriptState = { status: 'current', version: frame.version }
        this.removeWrittenSteers(true)
        this.emit({
          type: 'transcript_reset',
          start: frame.start,
          length: frame.length,
          blocks: frame.blocks,
          versions: frame.versions ?? [],
          failures: frame.failures ?? {},
          asked: this.asked,
        })
        this.followSends()
        return
      case 'transcript_patch':
        this.patchTranscript(frame)
        return
      case 'phase': {
        // A subagent's phase belongs to its record, which the host keeps;
        // only a compact of that child follows it.
        if (frame.subagentId !== undefined) {
          const previous = this.subagentPhases.get(frame.subagentId) ?? null
          this.subagentPhases.set(frame.subagentId, frame.phase)
          this.emit(frame)
          this.followPhase(frame.subagentId, previous, frame.phase)
          return
        }
        const previous = this.phase
        this.phase = frame.phase
        this.emit(frame)
        this.followPhase(null, previous, frame.phase)
        return
      }
      case 'queue':
        this.queue = frame.queue
        this.emit(frame)
        this.followSends()
        return
      case 'pending_steers':
        this.pending = structuredClone(frame.pendingSteers)
        this.removeWrittenSteers(false)
        this.emitPendingSteers()
        return
      case 'pending_calls':
        if (frame.subagentId === undefined) {
          this.calls = structuredClone(frame.pendingCalls)
          this.emit({ type: 'pending_calls', pendingCalls: this.pendingCalls() })
        } else {
          this.emit({ type: 'pending_calls', subagentId: frame.subagentId, pendingCalls: structuredClone(frame.pendingCalls) })
        }
        return
      case 'waiting': {
        const since = frame.waitedMs === null ? null : Date.now() - frame.waitedMs
        if (frame.subagentId === undefined) {
          this.emit({ type: 'waiting', since })
        } else {
          this.emit({ type: 'waiting', subagentId: frame.subagentId, since })
        }
        return
      }
      case 'steer_result':
        this.emit(frame)
        this.settleSteer(frame)
        return
      case 'abort_result':
        this.emit(frame)
        this.abortWaiters.shift()?.resolve(frame.result)
        return
      case 'subagent_transcript_reset':
        if (this.hidden.has(frame.subagentId)) {
          return
        }
        this.subagentRevisions.set(frame.subagentId, frame.revision)
        this.emit({
          type: 'subagent_transcript_reset',
          subagentId: frame.subagentId,
          start: frame.start,
          length: frame.length,
          blocks: frame.blocks,
          failures: frame.failures ?? {},
        })
        return
      case 'subagent_transcript_patch':
        this.patchSubagentTranscript(frame)
        return
      case 'rejected':
        this.emit(frame)
        this.rejectAction(frame.command, new RefusedError(frame))
        if (frame.command === 'abort') {
          for (const waiter of this.abortWaiters.splice(0)) {
            waiter.reject(new RefusedError(frame))
          }
        }
        return
      case 'error': {
        this.emit(frame)
        // A turn that failed is a transcript record; the wait learns it by the error's type.
        const error = new SessionError(frame)
        // A child's failure is a record in its transcript, never an `error` frame.
        const active = this.actionWaiters.find((waiter) => waiter.node === null && waiter.status === 'running')
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
        this.writtenTurns.clear()
        this.writtenSteers.clear()
        this.subagentRevisions.clear()
        this.subagentPhases.clear()
        this.phase = null
        this.queue = []
        this.pending = []
        this.calls = []
        this.emit(frame)
        for (const waiter of this.actionWaiters.splice(0)) {
          waiter.resolve()
        }
        this.rejectSteersAndAborts(new Error('Session closed'))
        return
      case 'edit_result':
      case 'remove_result':
      case 'shell_output':
      case 'subagents':
      case 'shell_write_result':
      case 'retry_scheduled':
      case 'context_usage':
      case 'cost':
      case 'cost_rows':
        this.emit(frame)
        return
      case 'subagent':
        this.emit(frame)
        // A closed child runs nothing more: what waited on it has ended.
        if (frame.event === 'closed') {
          const node = frame.job.subagentId
          this.subagentPhases.delete(node)
          for (const waiter of this.actionWaiters.filter((candidate) => candidate.node === node)) {
            this.settleAction(waiter, () => waiter.resolve())
          }
        }
        return
      // A heartbeat only shows that the socket is alive
      // (`rendering.md` § Order and delivery); it changes nothing here.
      case 'heartbeat':
        return
      // The answer to the page's ping as it returns, for the socket's
      // liveness watch.
      case 'pong':
        this.emit(frame)
        return
    }
  }

  /**
   * Applies a patch whose revision is one past the client's, ignores one that
   * is not past it, and asks for a fresh transcript at a gap
   * (`rendering.md` § Order and delivery).
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
    this.noteWritten(frame.patches.flatMap((patch) => (patch.op === 'add' || patch.op === 'replace_block' ? [patch.value] : [])))
    this.removeWrittenSteers(true)
    this.emit({ type: 'transcript_patch', patches: frame.patches, failures: frame.failures ?? {} })
    this.followSends()
  }

  /** The root's rule, for each subagent's stream. */
  private patchSubagentTranscript(frame: ServerFrameOf<'subagent_transcript_patch'>): void {
    if (this.transcriptState.status === 'stale' || this.hidden.has(frame.subagentId)) {
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
    const held = this.held()
    this.asked = held?.from
    this.sendFrame({ type: 'sync_transcript', ...held })
  }

  /** Notes the messages and steers `blocks` write. */
  private noteWritten(blocks: readonly Block[]): void {
    for (const block of blocks) {
      if (block.type === 'user') {
        this.writtenTurns.add(block.turnId)
      } else if (block.type === 'steer') {
        this.writtenSteers.add(block.id)
      }
    }
  }

  /** Drops the pending steers the transcript now holds, by id. */
  private removeWrittenSteers(announce: boolean): void {
    if (this.pending.length === 0) {
      return
    }
    const remaining = this.pending.filter((steer) => !this.writtenSteers.has(steer.id))
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
      waiter.reject(new SteerRejectedError(frame.outcome))
    }
  }

  /**
   * The session shows `idle` only once nothing runs or waits, and an action
   * that follows another starts without an `idle` between them
   * (`runtime.md` § A turn). So an action other than a send starts when the
   * session leaves `idle`, and at `idle` every action the session took has
   * ended.
   */
  private followPhase(node: string | null, previous: SessionPhase | null, phase: SessionPhase): void {
    const own = this.actionWaiters.filter((waiter) => waiter.node === node)
    if (phase !== 'idle') {
      if (previous === 'idle' || previous === null) {
        const next = own.find((waiter) => waiter.command !== 'send' && waiter.status === 'sent')
        if (next) {
          next.status = 'running'
        }
      }
      return
    }
    for (const waiter of own.filter((candidate) => candidate.status !== 'sent')) {
      this.settleAction(waiter, () => waiter.resolve())
    }
  }

  /**
   * A send is queued once the queue holds its message, and runs once its
   * message is written. The session runs one action at a time, so a message
   * that starts to run ends the action that ran before it.
   */
  private followSends(): void {
    const queued = new Set(this.queue.map((message) => message.id))
    const written = this.writtenTurns
    const waiting = this.actionWaiters.filter((waiter) => waiter.command === 'send' && waiter.status !== 'running')
    for (const waiter of waiting) {
      if (waiter.messageId !== null && written.has(waiter.messageId)) {
        const ended = this.actionWaiters.filter((candidate) => candidate.node === null && candidate.status === 'running')
        for (const previous of ended) {
          this.settleAction(previous, () => previous.resolve())
        }
        waiter.status = 'running'
      } else if (waiter.messageId !== null && queued.has(waiter.messageId)) {
        waiter.status = 'queued'
      }
    }
  }

  /** A refused command fails the first wait for it that the session has not taken, else the first. */
  private rejectAction(command: string, error: Error): void {
    const waiter = this.actionWaiters.find((candidate) => candidate.command === command && candidate.status === 'sent')
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
