import {
  serverFrameSchema,
  type AbortResult,
  type AgentEntry,
  type Block,
  type ClientContent,
  type ClientFrame,
  type EditRequest,
  type HeldEdge,
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
  type Version,
  schemaRefusal,
} from '@demicodes/protocol'
import { MAX_PAGE_MESSAGE_BYTES } from '@demicodes/protocol'
import { asError, createId } from '@demicodes/utils'
import type { ConversationClientListener, ClientSessionEvent, ServerFrameOf } from './events'
import { FrameTooLargeError, type ConversationClientTransport } from './transport'

/**
 * A frame the backend or the session refused. It carries the code the page words
 * the refusal by, the values that text needs, the refusal in English as the
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
 * A wait for the action a frame starts. It is `sent` until the session takes the
 * action. Then:
 *
 * - A send is `queued` while it waits behind another action, and `running` once
 *   its message is written.
 * - Any other action is `running` once the session leaves `idle` after it was
 *   sent.
 */
interface ActionWaiter {
  command: ActionCommand
  messageId: string | null
  /** The node whose action it is, whose phase it follows; null for the root's. */
  node: string | null
  status: 'sent' | 'queued' | 'running'
  resolve: () => void
  reject: (error: Error) => void
}

/** Which of the root's waiting inputs an agent entry changed. */
interface WaitingChanged {
  queue: boolean
  pending: boolean
  starting: boolean
}

interface Waiter<T> {
  resolve: (value: T) => void
  reject: (error: Error) => void
}

/**
 * Where the client's copy of a node's transcript is: none yet, current at a
 * version, or stale after a gap in the patch revisions, until the reset its
 * subscription asked for again arrives.
 */
type TranscriptState =
  | { status: 'none' }
  | { status: 'current'; version: Version }
  | { status: 'stale' }

/** A node the page subscribes to, and where its transcript is. */
interface Subscription {
  state: TranscriptState
  /** The index its last subscription named; a reset elsewhere means a rewrite. */
  asked: number | undefined
}

/** What the client is constructed with beside its transport. */
export interface ConversationClientOptions {
  /**
   * The frames of the conversation itself that the backend sends on the
   * socket beside its agents', such as its metadata, which the web app
   * decodes: answers whether it took `frame`. A frame neither the protocol nor
   * it takes drops the connection.
   */
  conversationFrame?: (frame: unknown) => boolean
}

/** What a wait decides about one event: to settle, or to keep waiting. */
type Decision<T> = { resolve: T } | { reject: Error } | undefined

/**
 * The web app's client of the conversation socket's frame protocol.
 *
 * It does four things:
 *
 * - Validates every frame it receives with the generated schemas, and drops the
 *   connection when one does not match.
 * - Keeps each subscribed node's patch revisions, the agents, the queue and the
 *   pending steers.
 * - Tells its listeners what each node's transcript frames bring, which the
 *   page applies to the parts of that transcript it holds with the one patch
 *   applier.
 * - Keeps, of the transcript, only which messages and steers it has seen written,
 *   which its waits settle on.
 */
export class ConversationClient {
  private readonly transport: ConversationClientTransport
  private readonly listeners = new Set<ConversationClientListener>()
  private readonly actionWaiters: ActionWaiter[] = []
  private readonly abortWaiters: Waiter<AbortResult>[] = []
  private readonly detachTransport: () => void
  private disconnected = false
  /** The nodes the page subscribes to, the root's included. */
  private readonly subscriptions = new Map<string, Subscription>()
  /** The root's turns of the `user` blocks the stream has brought since its last reset. */
  private readonly writtenTurns = new Set<string>()
  /** The ids of the root's `steer` blocks the stream has brought since its last reset. */
  private readonly writtenSteers = new Set<string>()
  /** The blocks the page holds of each node it subscribes to. */
  private held: (node: string) => HeldEdge | undefined = () => undefined
  /** The conversation's agents by id, as the last frames left them. */
  private readonly agents = new Map<string, AgentEntry>()
  /** The version of the agents the client holds; null before the open's. */
  private agentsVersion: Version | null = null
  /** The client asked for the agents' changes it missed, and ignores changes until the answer. */
  private agentsStale = false
  /** The root's node, the conversation's id; null before `open`. */
  private root: string | null = null
  private queue: QueuedMessage[] = []
  private pending: PendingSteer[] = []
  /** The message the root's running turn started with, until its turn writes it. */
  private starting: QueuedMessage | null = null
  private readonly conversationFrame: (frame: unknown) => boolean

  constructor(transport: ConversationClientTransport, options: ConversationClientOptions = {}) {
    this.transport = transport
    this.conversationFrame = options.conversationFrame ?? (() => false)
    const stopFrames = transport.onFrame((frame) => this.receive(frame))
    const stopClose = transport.onClose((error) => this.disconnect(error))
    this.detachTransport = () => {
      stopFrames()
      stopClose()
    }
  }

  /**
   * Attaches to the conversation's tree, which runs with the model settings
   * the conversation's record holds; resolves on `opened`. `root` is the
   * root's node, the conversation's id. The open names the agents the page
   * holds, `agents`, and the revision of the draft it holds, `draft`, which
   * it answers with what changed since. It subscribes to the root and to
   * `nodes` first, so its handshake brings their messages. `held` names the
   * blocks the page holds of each node it subscribes to, now and at each
   * later subscription again, so a reset sends none before them.
   */
  open(options: {
    root: string
    nodes?: readonly string[]
    held?: (node: string) => HeldEdge | undefined
    agents?: Version
    draft?: number
  }): Promise<void> {
    const { root, agents, draft } = options
    this.root = root
    this.held = options.held ?? (() => undefined)
    for (const node of [root, ...(options.nodes ?? [])]) {
      this.subscribeTo(node)
    }
    return this.request({ type: 'open', ...(agents ? { agents: { ...agents } } : {}), ...(draft === undefined ? {} : { draft }) }, (event) => {
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
    // The answer names the agents' version that shows the message, queued or
    // starting the turn; the send settles once the client holds it
    // (`docs/decisions/0023-a-command-settles-on-the-state-it-changed.md`).
    let at: Version | null | undefined
    return this.request(
      { type: 'send', messageId, content, ...(comments.length > 0 ? { comments: [...comments] } : {}) },
      (event) => {
        if (event.type === 'send_result' && event.messageId === messageId) {
          at = event.at
        }
        if (at !== undefined && this.holdsVersion(at)) {
          return { resolve: undefined }
        }
        return this.failure(event, 'send')
      },
    )
  }

  /**
   * Sends the message saved for later `id` the way the composer chose, as the
   * message `turnId`, the page's id for this send: as the list holds it, or as
   * an edit changed it, `content`. Resolves once the session holds
   * it: in the transcript or the queue, or accepted as a steer, at once when it
   * does already. The wait has no deadline of its own.
   *
   * It rejects with:
   *
   * - `SteerRejectedError` for a steer the session refuses.
   * - `SavedGoneError` for a message the list no longer holds, which no send took
   *   as `turnId`; never for an edited one, which goes as edited.
   * - `RefusedError` for any other refusal of the backend's.
   */
  sendSaved(id: string, turnId: string, delivery: SavedDelivery, content?: ClientContent[]): Promise<void> {
    if (this.writtenTurns.has(turnId) || this.queue.some((message) => message.id === turnId)) {
      return Promise.resolve()
    }
    // Settles once the client holds the agents' version the answer names.
    let at: Version | null | undefined
    return this.request(
      { type: 'send_saved', id, turnId, delivery, ...(content ? { content } : {}) },
      (event) => {
        if (event.type === 'steer_result' && event.steerId === turnId) {
          if (event.outcome.status !== 'accepted') {
            return { reject: new SteerRejectedError(event.outcome) }
          }
          at = event.outcome.at
        }
        if (event.type === 'send_result' && event.messageId === turnId) {
          at = event.at
        }
        if (at !== undefined && this.holdsVersion(at)) {
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
   * Regenerate: a new version of the answer to the message `request`
   * names; resolves on its durable receipt, as an edit does.
   */
  regenerate(request: RegenerateRequest): Promise<void> {
    return this.editRequest({ type: 'regenerate', request }, request.operationId)
  }

  /**
   * Makes the `to`th version at the place of the block `blockId`
   * current, from the transcript `version` the page shows. The rewrite
   * reaches the transcript as patches; a refusal arrives as a `rejected`
   * frame.
   */
  switchVersion(blockId: string, to: number, version: Version): void {
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
   * or `gone_on` when its turn had started or the session never held it.
   */
  async dequeueMessage(messageId: string): Promise<RemoveOutcome> {
    const { outcome, at } = await this.removeRequest(messageId, { type: 'dequeue_message', messageId })
    await this.untilState(() => this.holdsVersion(at))
    if (outcome === 'removed') {
      this.settleQueuedSend(messageId)
    }
    return outcome
  }

  /**
   * Sends a queued message now: while a turn runs, the turn ends at once and
   * the message runs next; otherwise it moves to the front of the queue. The
   * caller watches the queue and the transcript.
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

  /**
   * Adds input to the running turn; resolves once the client holds the agents'
   * version the answer names, which shows the steer, and rejects with
   * `SteerRejectedError` when the session refuses it
   * (`docs/decisions/0023-a-command-settles-on-the-state-it-changed.md`).
   */
  steer(content: ClientContent[], steerId: string = createId(), comments: readonly MessageComment[] = []): Promise<void> {
    let at: Version | null | undefined
    return this.request(
      { type: 'steer', steerId, content, ...(comments.length > 0 ? { comments: [...comments] } : {}) },
      (event) => {
        if (event.type === 'steer_result' && event.steerId === steerId) {
          if (event.outcome.status !== 'accepted') {
            return { reject: new SteerRejectedError(event.outcome) }
          }
          at = event.outcome.at
        }
        if (at !== undefined && this.holdsVersion(at)) {
          return { resolve: undefined }
        }
        // A turn that fails once the steer was accepted writes it into the
        // transcript: the failure is the turn's, not the steer's.
        if (at !== undefined && event.type === 'error') {
          return undefined
        }
        return this.failure(event, 'steer')
      },
    )
  }

  /**
   * Delivers a pending steer now: the running call returns at once with its
   * command still running, and the turn goes on with the steer. The caller
   * watches the pending steers and the transcript.
   */
  steerNow(steerId: string): void {
    this.sendFrame({ type: 'steer_now', steerId })
  }

  /**
   * Withdraws a pending steer; resolves with the session's answer, `removed`,
   * or `gone_on` when the turn had taken it or the session never held it, once
   * the client holds the agents' version that shows it.
   */
  async cancelPendingSteer(steerId: string): Promise<RemoveOutcome> {
    const { outcome, at } = await this.removeRequest(steerId, { type: 'cancel_pending_steer', steerId })
    await this.untilState(() => this.holdsVersion(at))
    return outcome
  }

  /**
   * Sends a removal and waits for its `remove_result`: its outcome and the
   * agents' version that shows it, which the caller waits to hold. A failed
   * turn does not fail it: its `error` frame is the turn's, not the removal's.
   */
  private removeRequest(id: string, frame: ClientFrame): Promise<{ outcome: RemoveOutcome; at: Version | null }> {
    return this.request(
      frame,
      (event) => {
        if (event.type === 'remove_result' && event.id === id) {
          return { resolve: { outcome: event.outcome, at: event.at } }
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

  /**
   * Finishes a turn that did not finish, as the action `id`, which the wait it
   * begins carries; resolves when it ends.
   */
  resume(id: string): Promise<void> {
    return this.action('resume', null, { type: 'resume', id })
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
   * Shows the node `node`'s messages from now on: its transcript arrives as
   * a reset from the blocks the page holds of it, unless it holds the current
   * version, while the node is live, then as patches.
   */
  subscribeTo(node: string): void {
    if (this.subscriptions.has(node)) {
      return
    }
    const held = this.held(node)
    this.subscriptions.set(node, {
      state: held ? { status: 'current', version: held.version } : { status: 'none' },
      asked: held?.from,
    })
    this.sendFrame({ type: 'subscribe', node, ...(held ? { held } : {}) })
  }

  /** Stops showing the node `node`'s messages; a frame of it still on its way is dropped. */
  unsubscribeFrom(node: string): void {
    if (!this.subscriptions.delete(node)) {
      return
    }
    this.sendFrame({ type: 'unsubscribe', node })
  }

  /** Stops every live subagent; each settles through its own agent entry. */
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

  /** The version of the node `node`'s transcript the client holds, the root's by default, or null while it holds none it can vouch for. */
  transcriptVersion(node: string | null = this.root): Version | null {
    const state = node === null ? undefined : this.subscriptions.get(node)?.state
    if (this.disconnected || state?.status !== 'current') {
      return null
    }
    return { ...state.version }
  }

  /** The version of the agents the client holds; null before the open answered. */
  heldAgents(): Version | null {
    return this.agentsVersion ? { ...this.agentsVersion } : null
  }

  /** The accepted steers not yet in the transcript, as copies. */
  pendingSteers(): PendingSteer[] {
    return structuredClone(this.pending)
  }

  /** The message the root's running turn started with, until its turn writes it, as a copy. */
  startingMessage(): QueuedMessage | null {
    return structuredClone(this.starting)
  }

  /** The calls the root's model is writing, as copies. */
  pendingCalls(): PendingCall[] {
    const root = this.root === null ? undefined : this.agents.get(this.root)
    return structuredClone(root?.pendingCalls ?? [])
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

  /**
   * Resolves once `holds` does, as the client's state stands now or after any
   * later event; rejects when the connection ends first.
   */
  private untilState(holds: () => boolean): Promise<void> {
    if (holds()) {
      return Promise.resolve()
    }
    if (this.disconnected) {
      return Promise.reject(new Error('Agent connection closed'))
    }
    return new Promise((resolve, reject) => {
      const unsubscribe = this.subscribe((event) => {
        if (holds()) {
          unsubscribe()
          resolve()
        } else if (event.type === 'disconnected') {
          unsubscribe()
          reject(event.error)
        } else if (event.type === 'closed') {
          unsubscribe()
          reject(new Error('Agent connection closed'))
        }
      })
    })
  }

  /**
   * Whether the client holds the agents at `at`, the version a command's
   * answer names: at or past its revision, or at another epoch, whose reset
   * showed the whole state; null names no tree, and nothing to wait for.
   */
  private holdsVersion(at: Version | null): boolean {
    const held = this.agentsVersion
    return at === null || (held !== null && (held.epoch !== at.epoch || held.revision >= at.revision))
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
    this.rejectAborts(error)
  }

  private rejectAborts(error: Error): void {
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
      if (this.conversationFrame(frame)) {
        return
      }
      this.disconnect(new Error(`Invalid server frame:\n${schemaRefusal(parsed.error)}`))
      return
    }
    this.handle(parsed.data)
  }

  private handle(frame: ServerFrame): void {
    switch (frame.type) {
      // The agents that follow tell the root's waiting inputs: its entry, or
      // nothing when it did not change since what the client holds.
      case 'opened':
        this.agentsStale = false
        this.emit(frame)
        return
      case 'transcript_reset': {
        const subscription = this.subscriptions.get(frame.node)
        if (!subscription) {
          return
        }
        subscription.state = { status: 'current', version: frame.version }
        const root = frame.node === this.root
        if (root) {
          this.writtenTurns.clear()
          this.writtenSteers.clear()
          this.noteWritten(frame.blocks)
          this.removeWrittenSteers(true)
        }
        this.emit({
          type: 'transcript_reset',
          node: frame.node,
          start: frame.start,
          length: frame.length,
          blocks: frame.blocks,
          version: frame.version,
          versions: frame.versions ?? [],
          failures: frame.failures ?? {},
          asked: subscription.asked,
        })
        if (root) {
          this.followSends()
        }
        return
      }
      case 'transcript_patch':
        this.patchTranscript(frame)
        return
      case 'agents': {
        const held = this.agentsVersion
        if (frame.since === undefined || held === null || held.epoch !== frame.version.epoch) {
          this.agents.clear()
          this.queue = []
          this.pending = []
          this.starting = null
        }
        this.agentsStale = false
        for (const agent of frame.agents) {
          this.takeAgent(agent)
        }
        this.agentsVersion = frame.version
        this.emit({ ...frame, receivedAt: Date.now() })
        // The waiting inputs as they stand now, which the caller takes as
        // listed even when the root's entry did not change.
        this.emitWaiting({ queue: true, pending: true, starting: true })
        return
      }
      case 'agent': {
        // Changes that arrive while the client asks for those it missed are
        // in the answer.
        if (this.agentsStale) {
          return
        }
        const held = this.agentsVersion
        if (held === null) {
          this.disconnect(new Error(`An agent change arrived at revision ${frame.revision} before the agents`))
          return
        }
        if (frame.revision !== held.revision + 1) {
          if (frame.revision > held.revision) {
            this.agentsStale = true
            this.sendFrame({ type: 'agents_since', held: { ...held } })
          }
          return
        }
        this.agentsVersion = { epoch: held.epoch, revision: frame.revision }
        const changed = this.takeAgent(frame.agent)
        this.emit({ ...frame, receivedAt: Date.now() })
        this.emitWaiting(changed)
        return
      }
      case 'request_output': {
        const arrived = Date.now()
        const output = frame.output
        this.emit({
          type: 'request_output',
          output: output === null
            ? null
            : {
                requestId: output.requestId,
                tokens: output.tokens,
                firstTokenAt: output.streamedMs === null ? null : arrived - output.streamedMs,
                endedAt: output.ended ? arrived : null,
              },
        })
        return
      }
      case 'steer_result':
      case 'send_result':
        this.emit(frame)
        return
      case 'abort_result':
        this.emit(frame)
        this.abortWaiters.shift()?.resolve(frame.result)
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
        this.rejectAborts(error)
        return
      }
      case 'closed':
        for (const subscription of this.subscriptions.values()) {
          subscription.state = { status: 'none' }
        }
        this.writtenTurns.clear()
        this.writtenSteers.clear()
        this.agents.clear()
        this.agentsVersion = null
        this.queue = []
        this.pending = []
        this.starting = null
        this.emit(frame)
        for (const waiter of this.actionWaiters.splice(0)) {
          waiter.resolve()
        }
        this.rejectAborts(new Error('Session closed'))
        return
      case 'edit_result':
      case 'remove_result':
      case 'shell_output':
      case 'cache_chart':
      case 'shell_write_result':
      case 'retry_scheduled':
        this.emit(frame)
        return
      // A heartbeat only shows that the socket is alive; it changes
      // nothing here.
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
   * Takes `agent` as its entry: its phase moves the waits of its actions on,
   * and a closed child's end every wait for it.
   */
  /**
   * Takes `agent` as its node's entry; answers which of the root's waiting
   * inputs changed with it.
   */
  private takeAgent(agent: AgentEntry): WaitingChanged {
    const previous = this.agents.get(agent.id)
    this.agents.set(agent.id, agent)
    const changed = { queue: false, pending: false, starting: false }
    if (agent.role.kind === 'root') {
      changed.queue = JSON.stringify(this.queue) !== JSON.stringify(agent.queue)
      changed.pending = JSON.stringify(this.pending) !== JSON.stringify(agent.pendingSteers)
      changed.starting = JSON.stringify(this.starting) !== JSON.stringify(agent.starting)
      this.queue = agent.queue
      this.pending = structuredClone(agent.pendingSteers)
      this.starting = agent.starting
      this.removeWrittenSteers(false)
    }
    const node = agent.role.kind === 'root' ? null : agent.id
    if (previous?.phase !== agent.phase) {
      this.followPhase(node, previous?.phase ?? null, agent.phase)
    }
    // A closed child runs nothing more: what waited on it has ended.
    if (agent.role.kind === 'subagent' && agent.role.state !== 'running') {
      for (const waiter of this.actionWaiters.filter((candidate) => candidate.node === agent.id)) {
        this.settleAction(waiter, () => waiter.resolve())
      }
    }
    return changed
  }

  /** Tells the root's waiting inputs that `changed`. */
  private emitWaiting(changed: WaitingChanged): void {
    if (changed.queue) {
      this.emit({ type: 'queue', queue: this.queue })
      this.followSends()
    }
    if (changed.pending) {
      this.emitPendingSteers()
    }
    if (changed.starting) {
      this.emit({ type: 'starting', starting: this.starting })
    }
  }

  /**
   * Applies a patch whose revision is one past the client's, ignores one that
   * is not past it or of a node the page does not subscribe to, and
   * subscribes again at a gap.
   */
  private patchTranscript(frame: ServerFrameOf<'transcript_patch'>): void {
    const subscription = this.subscriptions.get(frame.node)
    if (!subscription) {
      return
    }
    const state = subscription.state
    if (state.status === 'stale') {
      return
    }
    if (state.status === 'current' && frame.revision <= state.version.revision) {
      return
    }
    if (state.status === 'none' || frame.revision !== state.version.revision + 1) {
      this.resubscribe(frame.node, subscription)
      return
    }
    subscription.state = { status: 'current', version: { epoch: state.version.epoch, revision: frame.revision } }
    const root = frame.node === this.root
    if (root) {
      this.noteWritten(frame.patches.flatMap((patch) => (patch.op === 'add' || patch.op === 'replace_block' ? [patch.value] : [])))
      this.removeWrittenSteers(true)
    }
    this.emit({
      type: 'transcript_patch',
      node: frame.node,
      patches: frame.patches,
      version: { ...subscription.state.version },
      failures: frame.failures ?? {},
    })
    if (root) {
      this.followSends()
    }
  }

  /** A patch was missed: the client subscribes again, naming what it holds, and ignores patches until the reset arrives. */
  private resubscribe(node: string, subscription: Subscription): void {
    subscription.state = { status: 'stale' }
    const held = this.held(node)
    subscription.asked = held?.from
    this.sendFrame({ type: 'subscribe', node, ...(held ? { held } : {}) })
  }

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

  /**
   * The session shows `idle` only once nothing runs or waits, and an action
   * that follows another starts without an `idle` between them. So an
   * action other than a send starts when the session leaves `idle`, and at
   * `idle` every action the session took has ended.
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
