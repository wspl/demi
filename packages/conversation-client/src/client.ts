import {
  serverFrameSchema,
  type AbortResult,
  type AgentEntry,
  type Block,
  type ClientContent,
  type ClientFrame,
  type EditRequest,
  type Follow,
  type Followed,
  type MessageComment,
  type PendingCall,
  type PendingSteer,
  type ProviderErrorDiagnostics,
  type QueuedMessage,
  type RegenerateRequest,
  type RequestOutput,
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
import type { HeldEdge, TranscriptRead } from './transcript'
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
 * Where the client's copy of a node's transcript is: none, as before the
 * node is read; being read, while the socket's patches of it are dropped, and
 * the views of its commands, its request's output and its chart that it pushes
 * are kept, to follow the page's own; or current at a version.
 */
type TranscriptState =
  | { status: 'none' }
  | { status: 'reading'; frames: ReadingFrame[] }
  | {
      status: 'current'
      version: Version
      /** Whether the client holds the node's request output and chart, as its read or a push brought them; not for a node shown from a copy on disk. */
      readings: boolean
    }

/** A frame about a node that arrives while the node's page is read. */
type ReadingFrame = ServerFrameOf<'shell_output'> | ServerFrameOf<'request_output'> | ServerFrameOf<'cache_chart'>

/** A node the page follows, and where its transcript is. */
interface Subscription {
  state: TranscriptState
  /** Resolves once the node's transcript is read. */
  ready: Promise<void>
  /** Resolves once the first read of it has ended, whether it succeeded or failed. */
  attempted: Promise<void>
  /** Stops its read when the page stops following the node, so that nothing is downloaded for a node nobody shows. */
  stop: AbortController
}

/** The entity tags the page holds of the conversation's own values, which it follows them by. */
export interface ConversationTags {
  metadata: string
  saved: string
  workflows: string
}

/** What the page reads of a conversation beside its transcripts. */
export interface ConversationRead {
  /** The version of the agents the entries are at. */
  agents: Version
  entries: AgentEntry[]
  /** The tags of what the read brought of the conversation itself. */
  tags: Partial<ConversationTags>
}

/**
 * How the client reads what it follows:
 * a read depends on no socket, and the socket, opened after the reads, brings
 * changes alone. A read that fails is tried again after a wait, except at the
 * open, which fails with it.
 */
export interface ConversationReader {
  /**
   * The conversation's agents and its root's request output, with the tags of
   * the metadata and the lists it brings. With `whole` false only the agents
   * are wanted: the metadata and the lists are followed by the socket and
   * are not applied again.
   */
  conversation(signal: AbortSignal, options: { whole: boolean }): Promise<ConversationRead>
  /**
   * The latest page of the node's transcript, with its version and live
   * commands; with `earlier`, the page before it comes as one run of blocks
   * with it.
   */
  transcript(node: string, signal: AbortSignal, options?: { earlier: boolean }): Promise<TranscriptRead>
}

/**
 * What a frame of the conversation itself comes to, as `conversationFrame`
 * decodes it: the change `apply` makes, and, for a value the page follows by
 * its tag, which value it is and the tag the frame carries, which the next
 * connection follows it from.
 */
export interface ConversationFrameTake {
  apply: () => void
  tagged?: { value: keyof ConversationTags; tag: string }
}

/** Opens the connection the client's frames go on; called after the reads, once per connection. */
export type ConversationConnect = (signal: AbortSignal) => Promise<ConversationClientTransport>

/** What the client is constructed with beside how it connects. */
export interface ConversationClientOptions {
  /**
   * The frames of the conversation itself that the backend sends on the
   * socket beside its agents', such as its metadata, which the web app
   * decodes: answers what the frame is, or null when the page does not take
   * it. A frame neither the protocol nor it takes drops the connection.
   */
  conversationFrame?: (frame: unknown) => ConversationFrameTake | null
  /** Reads what the client follows; required to `open`. */
  reader?: ConversationReader
  /**
   * Tells that a change of `node`'s transcript, or of the agents when `node`
   * is null, could not be applied and was read again, for the page to report.
   */
  unapplied?: (node: string | null, reason: 'gap' | 'epoch') => void
  /**
   * Writes a line of the page's log about a follow: what the backend answered
   * it with, and why a value is read again. Scalars only.
   */
  logged?: (event: string, fields: Record<string, string | number | boolean | null>) => void
  /**
   * How long a read again waits after the backend told it a version is
   * stale, in milliseconds: a random time of up to two seconds by default, so
   * that pages told at once do not read at once.
   */
  staleDelayMs?: () => number
}

/** What a client that opens again holds already, as a page shows it from a copy on disk. */
export interface ConversationHeld {
  /** The agents' version, and the entries the page shows. */
  agents: { version: Version; entries: AgentEntry[] } | null
  /** The version each node's latest window is current at. */
  nodes: Readonly<Record<string, Version>>
}

/** What a wait decides about one event: to settle, or to keep waiting. */
type Decision<T> = { resolve: T } | { reject: Error } | undefined

/**
 * The web app's client of the conversation socket's frame protocol.
 *
 * It does five things:
 *
 * - Validates every frame it receives with the generated schemas, and drops the
 *   connection when one does not match.
 * - Reads what it follows, the agents and each followed node's latest page,
 *   with no socket, then connects and opens the socket naming each value with
 *   the version it holds, so the backend sends only what changed; a version it
 *   no longer holds comes back `stale`, or `rebased` when the backend moved
 *   what it holds to a live version, and on a gap or another epoch the
 *   client reads again and follows again; the patches that arrive while it
 *   reads a followed node are kept and applied past the read.
 * - Survives its connection: a client that opens again after the connection
 *   was lost holds what it read, and follows it from there, with no read.
 * - Keeps each followed node's patch revisions, the agents, the queue and the
 *   pending steers.
 * - Tells its listeners what each node's transcript frames bring, which the
 *   page applies to the parts of that transcript it holds with the one patch
 *   applier.
 * - Keeps, of the transcript, only which messages and steers it has seen written,
 *   which its waits settle on.
 */
export class ConversationClient {
  private readonly connect: ConversationConnect
  /** The connection the frames go on; null while there is none, before the open and after a loss. */
  private transport: ConversationClientTransport | null = null
  private detachTransport: () => void = () => {}
  private readonly listeners = new Set<ConversationClientListener>()
  private readonly actionWaiters: ActionWaiter[] = []
  private readonly abortWaiters: Waiter<AbortResult>[] = []
  /** Whether the client was disposed: it opens no more. */
  private disposed = false
  /** The nodes the page follows, the root's included. */
  private readonly subscriptions = new Map<string, Subscription>()
  /** The root's turns of the `user` blocks the stream has brought since its last reset. */
  private readonly writtenTurns = new Set<string>()
  /** The ids of the root's `steer` blocks the stream has brought since its last reset. */
  private readonly writtenSteers = new Set<string>()
  /** The blocks the page holds of each node it subscribes to. */
  private held: (node: string) => HeldEdge | undefined = () => undefined
  /** The conversation's agents by id, as the last frames left them. */
  private readonly agents = new Map<string, AgentEntry>()
  /** The version of the agents the client holds; null before the first read. */
  private agentsVersion: Version | null = null
  /** The tags of the conversation's own values the client follows, as the last read or frame carried them. */
  private readonly tags: Partial<ConversationTags> = {}
  /** Whether the agents are being read, which drops what the socket brings of them. */
  private agentsReading = false
  /** The revision of the draft the page holds, which the open names. */
  private draft: number | undefined
  /** Ends the reads when the client is disposed. */
  private readonly detached = new AbortController()
  private readonly reader: ConversationReader | null
  private readonly unapplied: (node: string | null, reason: 'gap' | 'epoch') => void
  private readonly logged: (event: string, fields: Record<string, string | number | boolean | null>) => void
  private readonly staleDelayMs: () => number
  /** The root's node, the conversation's id; null before `open`. */
  private root: string | null = null
  private queue: QueuedMessage[] = []
  private pending: PendingSteer[] = []
  /** The message the root's running turn started with, until its turn writes it. */
  private starting: QueuedMessage | null = null
  private readonly conversationFrame: (frame: unknown) => ConversationFrameTake | null

  constructor(connect: ConversationConnect, options: ConversationClientOptions = {}) {
    this.connect = connect
    this.conversationFrame = options.conversationFrame ?? (() => null)
    this.reader = options.reader ?? null
    this.unapplied = options.unapplied ?? (() => {})
    this.logged = options.logged ?? (() => {})
    this.staleDelayMs = options.staleDelayMs ?? (() => Math.random() * 2_000)
  }

  /**
   * Opens the conversation: reads what the page does not hold yet, the agents
   * and the latest page of the root and of each of `nodes`, with no socket,
   * and takes each as it answers; then connects and opens the socket, which
   * attaches to the conversation's tree, which runs with the model settings
   * the conversation's record holds, naming each value held with its
   * version, so the backend sends what changed since. Resolves once `opened`
   * came. A client that opens again after its connection was lost holds what
   * it read, and reads nothing. `root` is the root's node, the conversation's
   * id. The open names `draft`, the revision of the draft the page holds, so
   * the socket sends the draft only once it is newer. `held` names the
   * blocks the page holds of each node it follows, now and at each later
   * read, which a page read vouches for when the transcript did not change
   * under them. `holding` is what the page shows already, as from a copy on
   * disk, which a client that holds nothing follows from, with no read.
   * `signal` ends the attempt.
   */
  async open(options: {
    root: string
    nodes?: readonly string[]
    held?: (node: string) => HeldEdge | undefined
    draft?: number
    holding?: ConversationHeld
    signal?: AbortSignal
  }): Promise<void> {
    const { root } = options
    if (!this.reader) {
      throw new Error('A client that opens needs a reader')
    }
    if (this.disposed) {
      throw new Error('Agent connection closed')
    }
    this.root = root
    this.held = options.held ?? (() => undefined)
    this.draft = options.draft
    if (options.holding && this.agentsVersion === null) {
      this.hold(options.holding)
    }
    const nodes = [root, ...(options.nodes ?? [])].filter((node, at, all) => all.indexOf(node) === at)
    const reads: Promise<void>[] = []
    if (this.agentsVersion === null) {
      reads.push(this.readConversation(false))
    }
    for (const node of nodes) {
      if (this.subscriptions.get(node)?.state.status !== 'current') {
        this.subscriptions.get(node)?.stop.abort()
        this.subscriptions.delete(node)
        reads.push(this.follow(node, false).ready)
      }
    }
    await Promise.all(reads)
    await this.attach(options.signal)
  }

  /** Holds what the page shows already, which the first open follows from. */
  private hold(holding: ConversationHeld): void {
    if (holding.agents) {
      for (const agent of holding.agents.entries) {
        this.takeAgent(agent)
      }
      this.agentsVersion = { ...holding.agents.version }
      this.emitWaiting({ queue: true, pending: true, starting: true })
    }
    for (const [node, version] of Object.entries(holding.nodes)) {
      this.subscriptions.set(node, { state: { status: 'current', version: { ...version }, readings: false }, ready: Promise.resolve(), attempted: Promise.resolve(), stop: new AbortController() })
    }
  }

  /** Connects, and opens the socket with the versions held; the reads came before it. */
  private async attach(signal?: AbortSignal): Promise<void> {
    const ended = new AbortController()
    const end = () => ended.abort(signal?.reason ?? this.detached.signal.reason)
    signal?.addEventListener('abort', end, { once: true })
    this.detached.signal.addEventListener('abort', end, { once: true })
    try {
      const transport = await this.connect(ended.signal)
      if (this.disposed || ended.signal.aborted) {
        transport.close()
        throw ended.signal.reason ?? new Error('Agent connection closed')
      }
      const stopFrames = transport.onFrame((frame) => this.receive(frame))
      const stopClose = transport.onClose((error) => this.disconnect(error))
      this.transport = transport
      this.detachTransport = () => {
        stopFrames()
        stopClose()
      }
      const draft = this.draft
      const opening = { type: 'open' as const, ...(draft === undefined ? {} : { draft }), ...this.followsOf() }
      this.logged('socket.followed', { values: opening.follows?.length ?? 0, nodes: this.subscriptions.size })
      await this.request(
        opening,
        (event) => (event.type === 'opened' ? { resolve: undefined } : this.failure(event, 'open')),
      )
    } finally {
      signal?.removeEventListener('abort', end)
      this.detached.signal.removeEventListener('abort', end)
    }
  }

  /** The values the client holds, each with its version, for the open to follow. */
  private followsOf(): { follows?: Follow[] } {
    const follows: Follow[] = (['metadata', 'saved', 'workflows'] as const).map((value) => ({
      value: { kind: value },
      holds: { kind: 'tag', tag: this.tags[value] ?? '' },
    }))
    if (this.agentsVersion) {
      follows.push({ value: { kind: 'agents' }, holds: { kind: 'version', version: { ...this.agentsVersion } } })
    }
    for (const [node, subscription] of this.subscriptions) {
      if (subscription.state.status === 'current') {
        follows.push(this.transcriptFollow(node, subscription.state.version, subscription.state.readings))
      }
    }
    return { follows }
  }

  private transcriptFollow(node: string, version: Version, readings: boolean): Follow {
    return { value: { kind: 'transcript', node }, holds: { kind: 'version', version: { ...version }, ...(readings ? { readings } : {}) } }
  }

  /** Follows `value` from `version` on the open connection; a client with none follows it at its next open. */
  private sendFollow(value: Followed, version: Version, readings = false): void {
    this.sendFrame({ type: 'follow', value, holds: { kind: 'version', version: { ...version }, ...(readings ? { readings } : {}) } })
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
    // starting the turn; the send settles once the client holds it.
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
   * `SteerRejectedError` when the session refuses it.
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
    if (!this.transport) {
      return Promise.reject(new Error('Agent connection closed'))
    }
    return new Promise((resolve, reject) => {
      this.abortWaiters.push({ resolve, reject })
      this.sendFrame({ type: 'abort' })
    })
  }

  /**
   * Shows the node `node`'s messages from now on: reads its latest page, which
   * the page holds as a reset, then follows the node from the version the read
   * names. A read that fails is tried again after a wait. Resolves once it
   * holds the read, or with `until` `attempt`, once the first read has ended,
   * for a caller that goes on to the next node while a failed one is tried
   * again.
   */
  followNode(node: string, until: 'read' | 'attempt' = 'read'): Promise<void> {
    const subscription = this.follow(node, true)
    return until === 'read' ? subscription.ready : subscription.attempted
  }

  /** Reads `node` and follows it; `retrying` says whether a failed read is tried again. */
  private follow(node: string, retrying: boolean): Subscription {
    const held = this.subscriptions.get(node)
    if (held) {
      return held
    }
    let attempted = () => {}
    const subscription: Subscription = {
      state: { status: 'reading', frames: [] },
      ready: Promise.resolve(),
      attempted: new Promise<void>((resolve) => {
        attempted = resolve
      }),
      stop: new AbortController(),
    }
    this.subscriptions.set(node, subscription)
    subscription.ready = this.readNode(node, subscription, retrying, false, attempted)
    if (retrying) {
      // The caller that wants the outcome holds the promise; one that does
      // not hears nothing of a connection that ended.
      subscription.ready.catch(() => {})
    }
    return subscription
  }

  /** Stops following the node `node`'s messages; a frame of it still on its way is dropped. */
  unfollowNode(node: string): void {
    const subscription = this.subscriptions.get(node)
    if (!subscription) {
      return
    }
    this.subscriptions.delete(node)
    subscription.stop.abort()
    this.sendFrame({ type: 'unfollow', value: { kind: 'transcript', node } })
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

  /**
   * Lets the connection go without the frame that disposes the tree, which
   * keeps running: the waits for answers end, and the client holds what it
   * read, to follow it again from `open`.
   */
  disconnect(error: Error = new Error('Agent connection closed')): void {
    const transport = this.transport
    if (!transport) {
      return
    }
    this.transport = null
    this.detachTransport()
    transport.close()
    this.rejectWaiters(error)
    this.emit({ type: 'disconnected', error })
  }

  /** Lets the connection go and opens no more: what the client read is dropped with it. */
  dispose(error: Error = new Error('Agent connection closed')): void {
    this.disposed = true
    this.detached.abort()
    this.disconnect(error)
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
    if (!this.transport || state?.status !== 'current') {
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
    const transport = this.transport
    if (!transport) {
      return false
    }
    try {
      transport.send(frame)
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
    if (!this.transport) {
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
    if (!this.transport) {
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
    if (!this.transport) {
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
      const take = this.conversationFrame(frame)
      if (take) {
        if (take.tagged) {
          this.tags[take.tagged.value] = take.tagged.tag
        }
        take.apply()
        return
      }
      this.disconnect(new Error(`Invalid server frame:\n${schemaRefusal(parsed.error)}`))
      return
    }
    this.handle(parsed.data)
  }

  private handle(frame: ServerFrame): void {
    switch (frame.type) {
      case 'opened':
        this.emit(frame)
        return
      case 'transcript_patch':
        this.patchTranscript(frame)
        return
      case 'agents':
        this.changeAgents(frame)
        return
      case 'agent':
        this.changeAgent(frame)
        return
      case 'stale':
        this.stale(frame.value)
        return
      case 'rebased':
        this.rebase(frame.value, frame.version)
        return
      case 'request_output':
      case 'cache_chart': {
        // A frame pushed while its node's page is read goes after the page's
        // own, whichever of the two is older.
        const state = this.subscriptions.get(frame.node)?.state
        if (state?.status === 'reading') {
          state.frames.push(frame)
        } else {
          if (state?.status === 'current') {
            state.readings = true
          }
          this.emitFrame(frame)
        }
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
        this.agentsReading = false
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
      case 'shell_output': {
        // A command's view pushed while its node's page is read goes after
        // the page's own commands, whichever of the two is older.
        const state = this.subscriptions.get(frame.node)?.state
        if (state?.status === 'reading') {
          state.frames.push(frame)
        } else {
          this.emit(frame)
        }
        return
      }
      case 'edit_result':
      case 'remove_result':
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
   * The backend cannot bring a followed value current from the version the
   * client holds, and has ended the follow: the client reads the value again,
   * after a random wait so that pages told at once do not read at once, and
   * follows it from the version the read names.
   */
  private stale(value: Followed): void {
    this.logged('follow.stale', { value: value.kind, ...(value.kind === 'transcript' ? { node: value.node } : {}) })
    switch (value.kind) {
      case 'agents':
        this.readConversationAgain(null)
        return
      case 'transcript': {
        const subscription = this.subscriptions.get(value.node)
        if (subscription) {
          this.readAgain(value.node, subscription, null)
        }
        return
      }
      // The backend sends a whole value that differs, so nothing is stale.
      case 'metadata':
      case 'saved':
      case 'workflows':
      case 'shares':
        return
    }
  }

  /**
   * Moves the copy the page holds of `value`, read while no session lived, to
   * `version`, the live one: the backend found it what the store held when
   * its tree restored.
   */
  private rebase(value: Followed, version: Version): void {
    switch (value.kind) {
      case 'agents':
        if (this.agentsVersion !== null && !this.agentsReading) {
          this.agentsVersion = { ...version }
        }
        return
      case 'transcript': {
        const state = this.subscriptions.get(value.node)?.state
        if (state?.status === 'current') {
          state.version = { ...version }
        }
        return
      }
      case 'metadata':
      case 'saved':
      case 'workflows':
      case 'shares':
        return
    }
  }

  /**
   * Takes the agents' change `frame` when its revision is one past the
   * client's, ignores one that is not past what the client holds or that the
   * agents' read overtook, and reads the conversation again at a gap or
   * another epoch.
   */
  private changeAgent(frame: ServerFrameOf<'agent'>): void {
    const held = this.agentsVersion
    if (held === null || this.agentsReading) {
      return
    }
    if (frame.version.epoch !== held.epoch) {
      this.readConversationAgain('epoch')
      return
    }
    if (frame.version.revision <= held.revision) {
      return
    }
    if (frame.version.revision !== held.revision + 1) {
      this.readConversationAgain('gap')
      return
    }
    this.applyAgent(frame)
  }

  private applyAgent(frame: ServerFrameOf<'agent'>): void {
    this.agentsVersion = { ...frame.version }
    const changed = this.takeAgent(frame.agent)
    this.emit({ ...frame, receivedAt: Date.now() })
    this.emitWaiting(changed)
  }

  /**
   * Takes the entries the backend sends of a follow from the revision the
   * client held: those changed since, at the version they bring the agents
   * to; a revision that is not the one the client holds reads again.
   */
  private changeAgents(frame: ServerFrameOf<'agents'>): void {
    const held = this.agentsVersion
    if (held === null || this.agentsReading) {
      return
    }
    if (frame.version.epoch !== held.epoch) {
      this.readConversationAgain('epoch')
      return
    }
    if (frame.since === undefined || frame.since !== held.revision) {
      if (frame.version.revision > held.revision) {
        this.readConversationAgain('gap')
      }
      return
    }
    this.agentsVersion = { ...frame.version }
    const changed = { queue: false, pending: false, starting: false }
    const receivedAt = Date.now()
    for (const agent of frame.agents) {
      const moved = this.takeAgent(agent)
      changed.queue ||= moved.queue
      changed.pending ||= moved.pending
      changed.starting ||= moved.starting
      this.emit({ type: 'agent', version: { ...frame.version }, agent, receivedAt })
    }
    this.emitWaiting(changed)
  }

  /** Reads the conversation again: `reason` says what the page could not apply, or null when the backend said the version was stale. */
  private readConversationAgain(reason: 'gap' | 'epoch' | null): void {
    if (reason) {
      this.logged('read.again', { value: 'agents', reason })
      this.unapplied(null, reason)
    }
    if (this.agentsReading) {
      return
    }
    // The follow of the agents has ended, as the backend ended it or as the
    // page ends it now.
    if (reason) {
      this.sendFrame({ type: 'unfollow', value: { kind: 'agents' } })
    }
    this.agentsReading = true
    this.readConversation(true, reason === null).catch(() => {})
  }

  /**
   * Reads the conversation and takes it, then follows the agents from the
   * version it names when the connection is open. A failed read is tried
   * again after a wait, unless the open asks it once, which fails with it.
   */
  private async readConversation(retrying: boolean, staled = false): Promise<void> {
    const reader = this.reader!
    this.agentsReading = true
    if (staled) {
      await this.sleep(this.staleDelayMs())
    }
    for (let failures = 0; ; ) {
      if (this.disposed) {
        return
      }
      let read: ConversationRead
      try {
        read = await reader.conversation(this.detached.signal, { whole: this.agentsVersion === null })
      } catch (error) {
        if (this.disposed) {
          return
        }
        if (!retrying) {
          this.agentsReading = false
          throw error
        }
        failures += 1
        await this.wait(failures)
        continue
      }
      if (this.disposed) {
        return
      }
      this.adoptAgents(read)
      return
    }
  }

  /** Holds `read` as the agents and follows them from its version. */
  private adoptAgents(read: ConversationRead): void {
    this.agentsReading = false
    this.agents.clear()
    this.queue = []
    this.pending = []
    this.starting = null
    for (const agent of read.entries) {
      this.takeAgent(agent)
    }
    this.agentsVersion = { ...read.agents }
    Object.assign(this.tags, read.tags)
    this.emit({ type: 'agents', agents: read.entries, version: { ...read.agents }, receivedAt: Date.now() })
    // The waiting inputs as they stand now, which the caller takes as listed
    // even when the root's entry did not change.
    this.emitWaiting({ queue: true, pending: true, starting: true })
    this.sendFollow({ kind: 'agents' }, read.agents)
  }

  /** Tells the listeners a frame the client holds while a node's page is read, or as it arrives. */
  private emitFrame(frame: ReadingFrame): void {
    if (frame.type === 'request_output') {
      this.emitRequestOutput(frame.node, frame.output)
    } else {
      this.emit(frame)
    }
  }

  private emitRequestOutput(node: string, output: RequestOutput | null): void {
    const arrived = Date.now()
    this.emit({
      type: 'request_output',
      node,
      output: output === null
        ? null
        : {
            requestId: output.requestId,
            tokens: output.tokens,
            firstTokenAt: output.streamedMs === null ? null : arrived - output.streamedMs,
            endedAt: output.ended ? arrived : null,
          },
    })
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
    const changed = { queue: false, pending: false, starting: false }
    // A deleted agent is gone from the collection, as the whole frame has it.
    if (agent.deleted) {
      this.agents.delete(agent.id)
      return changed
    }
    this.agents.set(agent.id, agent)
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
   * is not past it, one of a node the page does not follow and one that comes
   * while the node is read, and reads the node again at a gap or another
   * epoch.
   */
  private patchTranscript(frame: ServerFrameOf<'transcript_patch'>): void {
    const subscription = this.subscriptions.get(frame.node)
    if (!subscription) {
      return
    }
    const state = subscription.state
    if (state.status !== 'current') {
      return
    }
    if (frame.version.epoch !== state.version.epoch) {
      this.readAgain(frame.node, subscription, 'epoch')
      return
    }
    if (frame.version.revision <= state.version.revision) {
      this.logged('transcript.patch_ignored', { node: frame.node, held: state.version.revision, arrived: frame.version.revision })
      return
    }
    if (frame.version.revision !== state.version.revision + 1) {
      this.readAgain(frame.node, subscription, 'gap')
      return
    }
    this.applyPatch(frame.node, subscription, frame)
  }

  /** Takes `frame`, the next patch of the node's transcript, as the page's. */
  private applyPatch(node: string, subscription: Subscription, frame: ServerFrameOf<'transcript_patch'>): void {
    const readings = subscription.state.status === 'current' && subscription.state.readings
    subscription.state = { status: 'current', version: { ...frame.version }, readings }
    const root = node === this.root
    if (root) {
      this.noteWritten(frame.patches.flatMap((patch) => (patch.op === 'add' || patch.op === 'replace_block' ? [patch.value] : [])))
      this.removeWrittenSteers(true)
    }
    this.emit({
      type: 'transcript_patch',
      node,
      patches: frame.patches,
      version: { ...frame.version },
      failures: frame.failures ?? {},
      attachments: frame.attachments ?? [],
    })
    if (root) {
      this.followSends()
    }
  }

  /**
   * Reads the node again: a patch the client cannot apply (`reason`), or the
   * backend's word that the version held is stale (null), which has ended the
   * follow already.
   */
  private readAgain(node: string, subscription: Subscription, reason: 'gap' | 'epoch' | null): void {
    if (subscription.state.status === 'reading') {
      return
    }
    if (reason) {
      this.logged('read.again', { value: 'transcript', node, reason })
      this.unapplied(node, reason)
      this.sendFrame({ type: 'unfollow', value: { kind: 'transcript', node } })
    }
    subscription.state = { status: 'reading', frames: [] }
    subscription.ready = this.readNode(node, subscription, true, reason === null)
    subscription.ready.catch(() => {})
  }

  /**
   * Reads the node's latest page and takes it, then follows the node from
   * the version it names when the connection is open. A failed read is tried
   * again after a wait, unless the open asks it once, which fails with it.
   */
  private async readNode(
    node: string,
    subscription: Subscription,
    retrying: boolean,
    staled = false,
    attempted: () => void = () => {},
  ): Promise<void> {
    try {
      const reader = this.reader!
      const live = () => !this.disposed && this.subscriptions.get(node) === subscription
      if (staled) {
        await this.sleep(this.staleDelayMs())
      }
      for (let failures = 0; ; ) {
        if (!live()) {
          return
        }
        let read: TranscriptRead
        try {
          // The root's first read of a page that holds none of it brings the
          // page before the latest too, which the view would ask for next.
          read = await reader.transcript(node, AbortSignal.any([this.detached.signal, subscription.stop.signal]), {
            earlier: node === this.root && this.held(node) === undefined,
          })
        } catch (error) {
          attempted()
          if (!live()) {
            return
          }
          if (!retrying) {
            throw error
          }
          failures += 1
          await this.wait(failures)
          continue
        }
        if (!live()) {
          return
        }
        this.adoptTranscript(node, subscription, read)
        return
      }
    } finally {
      // The success, and a node left before its read ended, settle it too.
      attempted()
    }
  }

  /** Waits `ms`, or until the client is disposed. */
  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => {
      const timer = setTimeout(resolve, ms)
      this.detached.signal.addEventListener('abort', () => {
        clearTimeout(timer)
        resolve()
      }, { once: true })
    })
  }

  /** Waits the backoff of the `failures`th failure in a row: one second doubling to thirty, each up to half shorter. */
  private wait(failures: number): Promise<void> {
    const longest = Math.min(30_000, 1_000 * 2 ** (failures - 1))
    return this.sleep(longest * (1 - Math.random() / 2))
  }

  /** Holds `read` as the node's transcript and follows the node from its version. */
  private adoptTranscript(node: string, subscription: Subscription, read: TranscriptRead): void {
    const buffered = subscription.state.status === 'reading' ? subscription.state.frames : []
    subscription.state = { status: 'current', version: { ...read.version }, readings: true }
    const root = node === this.root
    if (root) {
      this.writtenTurns.clear()
      this.writtenSteers.clear()
      this.noteWritten(read.blocks)
      this.removeWrittenSteers(true)
    }
    const held = this.held(node)
    // What the page holds before the page read stands when the transcript is
    // of the epoch it was held in and the read finds the edge block where the
    // page held it, so that nothing before it was rewritten meanwhile.
    const vouched =
      held !== undefined &&
      held.version.epoch === read.version.epoch &&
      held.from >= read.start &&
      read.blocks[held.from - read.start]?.id === held.edge
    this.logged('transcript.reset', {
      node,
      start: read.start,
      length: read.length,
      blocks: read.blocks.length,
      held: held !== undefined,
      vouched,
      buffered_frames: buffered.length,
      revision: read.version.revision,
    })
    this.emit({
      type: 'transcript_reset',
      node,
      start: read.start,
      length: read.length,
      blocks: read.blocks,
      version: { ...read.version },
      versions: read.versions ?? [],
      failures: read.failures ?? {},
      attachments: read.attachments ?? [],
      instructions: read.instructions ?? [],
      summaries: read.summaries ?? [],
      asked: vouched ? read.start : undefined,
    })
    for (const status of read.commands ?? []) {
      this.emit({ type: 'shell_output', node, status })
    }
    this.emitRequestOutput(node, read.requestOutput ?? null)
    if (read.cacheChart) {
      this.emit({ type: 'cache_chart', node, chart: read.cacheChart })
    }
    for (const frame of buffered) {
      if (frame.type === 'shell_output') {
        this.emit(frame)
      } else {
        this.emitFrame(frame)
      }
    }
    if (root) {
      this.followSends()
    }
    this.sendFollow({ kind: 'transcript', node }, read.version, true)
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
