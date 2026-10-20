import { shallowRef, triggerRef } from 'vue'
import { errorCode, shownError } from '../infra/errors'
import {
  ActionOutlivedError,
  SavedGoneError,
  RefusedError,
  SessionError,
  SteerRejectedError,
  applyTranscriptPatches,
  heldEdge,
  resetTranscript,
  windowOf,
  type ConversationClient,
  type ClientSessionEvent,
} from '@demicodes/conversation-client'
import { asError, createId, deferred } from '@demicodes/utils'
import type { ClientContent, EditRequest, MessageComment, RegenerateRequest, SavedDelivery, TranscriptVersion } from '@demicodes/protocol'
import { ConversationSocketError } from '../transport/conversation-socket'
import { pendingReturnCheck, waitToReconnect } from '../transport/liveness'
import { failureHeadline } from './error-detail'
import { hasAcceptedSubmission, isWritten } from './submission'
import type { ConversationState } from './types'

/**
 * A rejected action whose failure the session reported as an `error` frame:
 * a failed turn is a record in the transcript, and a refused frame or a
 * failed save is the session's `lastError`, so the caller shows nothing else.
 */
export function isRecordedTurnFailure(error: unknown): boolean {
  return error instanceof SessionError
}

/**
 * Waits for the end of an action the session runs (a retry, a resume or a
 * compaction). A connection that ends meanwhile, as when the page slept
 * through the turn, fails nothing: the session had taken the action, and the
 * next connection's snapshot shows how it went.
 */
async function untilEndedOrLost(ended: Promise<void>): Promise<void> {
  try {
    await ended
  } catch (error) {
    if (!(error instanceof ActionOutlivedError)) {
      throw error
    }
  }
}

/**
 * A Recall the session could not carry out, since the message went on
 * first: the agent read the steer, or the queued message's turn began. The
 * transcript holds the message, and the page shows it there with no toast; the
 * message is for the console.
 */
export class RemovalRefusedError extends Error {}

/** The refusal of a removal whose message went on: a steer the agent read, or a queued message whose turn began. */
function refusedRemoval(steer: boolean): RemovalRefusedError {
  return new RemovalRefusedError(steer ? 'The agent had already read it.' : 'Its turn had already begun.')
}

export type RuntimeState = Pick<
  ConversationState,
  | 'id'
  | 'history'
  | 'phase'
  | 'queue'
  | 'pendingSteers'
  | 'pendingCalls'
  | 'waitingSince'
  | 'lastError'
  | 'load'
  | 'pendingAction'
  | 'failures'
  | 'contextUsage'
>

export interface ConversationRuntimeOptions {
  state: RuntimeState
  connect: (signal: AbortSignal) => Promise<ConversationClient>
  onEvent?: (event: ClientSessionEvent) => void
  /**
   * The children whose transcripts the page shows now, which each connection
   * names before its `open` (`crates/agent-server/src/server/tree.rs`); `showSubagents`
   * names them again when they change.
   */
  shownSubagents?: () => readonly string[]
}

const OPEN_TIMEOUT_MS = 30_000

/**
 * Owns one reconnectable client. Disposing a view leaves its server task alive.
 *
 * A connection that cannot be made or is lost, before the session answered
 * `open` or after, or whose tree another client disposed, is never a
 * failure the reader is told about: on a weak network it comes and goes, so
 * the runtime keeps `load` at `reconnecting` (the transcript's tail row says
 * Connecting, the composer stays), or at `connecting` while the session has
 * never opened, and tries again after the page's reconnect
 * waits until the socket is back or the view is disposed. An action taken
 * meanwhile waits for the connection, without cutting a wait short. Only the
 * session refusing to open is a failure, told once through `load` `failed` and
 * `lastError`.
 */
export class ConversationRuntime {
  private readonly options: ConversationRuntimeOptions
  private readonly client = shallowRef<ConversationClient | null>(null)
  private opening: Promise<ConversationClient> | null = null
  private controller: AbortController | null = null
  private unsubscribe: (() => void) | null = null
  /**
   * The clients whose session has listed its pending steers, the last of the
   * frames that show the conversation as it opens.
   */
  private readonly listedSteers = new WeakSet<ConversationClient>()
  /** Consecutive connections lost or not made since the session last opened. */
  private connectionFailures = 0
  /** Whether a session of this view has opened: a later opening is a reconnection. */
  private opened = false
  private disposed = false

  constructor(options: ConversationRuntimeOptions) {
    this.options = options
  }

  get connected(): boolean {
    return this.client.value !== null
  }

  /** Whether a session of this view has opened, whose handshake the page has applied. */
  get hasOpened(): boolean {
    return this.opened
  }

  /**
   * Sends a message; resolves once the session holds it, in the transcript
   * or the queue. A connection lost before the session confirmed it fails
   * nothing: the send waits for the connection to come back, then sends the
   * message again with its id, which the session takes only once, unless
   * what the reopened conversation shows already holds it. `comments` are
   * those it carries.
   */
  async submit(content: ClientContent[], messageId: string = createId(), comments: readonly MessageComment[] = []): Promise<void> {
    this.options.state.lastError = null
    for (;;) {
      const client = await this.ensureOpen()
      if (hasAcceptedSubmission(this.options.state, messageId)) {
        return
      }
      try {
        await client.submit(content, messageId, comments)
        return
      } catch (error) {
        if (!(error instanceof ConversationSocketError)) {
          throw error
        }
      }
    }
  }

  /**
   * Steers the running turn with a message; resolves once the session holds it
   * as a pending steer. A steer the session refuses, because its turn is ending
   * or nothing runs, is sent as a message instead, with the same id and no
   * error. A connection lost before the session answered fails nothing: once
   * the conversation is open again and has listed its pending steers, a steer
   * it neither lists nor holds in the transcript is sent as a message, so it is
   * never lost and never delivered twice.
   */
  async steer(content: ClientContent[], id: string = createId(), comments: readonly MessageComment[] = []): Promise<void> {
    this.options.state.lastError = null
    const client = await this.ensureOpen()
    try {
      await client.steer(content, id, comments)
      return
    } catch (error) {
      if (error instanceof ConversationSocketError) {
        if (await this.holdsSteer(id)) {
          return
        }
      } else if (!(error instanceof SteerRejectedError)) {
        throw error
      }
    }
    await this.submit(content, id, comments)
  }

  /**
   * Sends the message saved for later `id` the way the composer chose, as
   * the message `turnId`; the backend takes it off the list once the session
   * takes it, and keeps it when the session refuses it. A connection lost
   * before the session answered fails nothing: once the conversation is open
   * again, a send the session holds already is not sent again, and one it
   * does not hold goes again as the same turn. A steer the session refuses,
   * as its turn ended meanwhile, goes as a message, as the composer's steer
   * would. Answers false when the list no longer held it, as another page
   * sent or deleted it first, and nothing was sent.
   */
  async sendSaved(id: string, turnId: string, delivery: SavedDelivery): Promise<boolean> {
    this.options.state.lastError = null
    let way = delivery
    for (;;) {
      const client = await this.ensureOpen()
      if (hasAcceptedSubmission(this.options.state, turnId)) {
        return true
      }
      try {
        await client.sendSaved(id, turnId, way)
        return true
      } catch (error) {
        if (error instanceof SavedGoneError) {
          return false
        }
        if (error instanceof SteerRejectedError) {
          way = 'message'
        } else if (!(error instanceof ConversationSocketError)) {
          throw error
        }
      }
    }
  }

  /** Whether the session, open again after a lost connection, holds the steer `id`: pending, or in the transcript. */
  private async holdsSteer(id: string): Promise<boolean> {
    for (;;) {
      const client = await this.ensureOpen()
      try {
        await this.steersListed(client)
      } catch (error) {
        if (!(error instanceof ConversationSocketError)) {
          throw error
        }
        continue
      }
      return hasAcceptedSubmission(this.options.state, id)
    }
  }

  /**
   * Resolves once `client`'s session has listed its pending steers, so the
   * page holds what the session holds; rejects when the connection ends first.
   */
  private async steersListed(client: ConversationClient): Promise<void> {
    if (this.listedSteers.has(client)) {
      return
    }
    if (this.client.value !== client) {
      throw new ConversationSocketError('The connection ended before the session listed its steers')
    }
    const listed = deferred()
    const unsubscribe = client.subscribe((event) => {
      if (event.type === 'pending_steers') {
        listed.resolve()
      } else if (event.type === 'disconnected' || event.type === 'closed') {
        listed.reject(new ConversationSocketError('The connection ended before the session listed its steers'))
      }
    })
    try {
      await listed.promise
    } finally {
      unsubscribe()
    }
  }

  transcriptVersion(): TranscriptVersion | null {
    return this.client.value?.transcriptVersion() ?? null
  }

  editAndSend(request: EditRequest): Promise<void> {
    return this.edit(request.targetBlockId, (client) => client.editAndSend(request))
  }

  /** Regenerate: a new version of the answer to the message `request` names, sent as an edit is. */
  regenerate(request: RegenerateRequest): Promise<void> {
    return this.edit(request.targetBlockId, (client) => client.regenerate(request))
  }

  /**
   * Shows the `to`th version at the place of the block `blockId`, from the
   * transcript the page shows; the path's new blocks arrive as the
   * transcript's patches. Nothing is sent before the transcript has a
   * version, which the controls cannot be shown without.
   */
  async switchVersion(blockId: string, to: number): Promise<void> {
    const client = await this.ensureOpen()
    const version = client.transcriptVersion()
    if (version) {
      client.switchVersion(blockId, to, version)
    }
  }

  private async edit(targetBlockId: string, send: (client: ConversationClient) => Promise<void>): Promise<void> {
    const client = await this.ensureOpen()
    const replacesVisibleTarget = windowOf(this.options.state.history, targetBlockId) !== undefined
    let receivedError = false
    const unsubscribe = client.subscribe((event) => {
      if (event.type === 'error') {
        receivedError = true
      }
    })
    try {
      await send(client)
      // Reconciliation can confirm an earlier edit after its generation failed.
      // That receipt must not erase the current turn's recovery action.
      if (replacesVisibleTarget && !receivedError) {
        this.options.state.lastError = null
      }
    } finally {
      unsubscribe()
    }
  }

  /**
   * Removes a steer or a queued message that waits for the agent; resolves once
   * the session answers it removed, and rejects with `RemovalRefusedError` when
   * the session answers that the message had gone on. A connection lost before
   * the answer fails nothing: once the conversation is open again and has
   * listed what waits, a message still listed is asked for again; one the
   * transcript holds went on, and one held nowhere was removed.
   */
  async removeWaiting(id: string): Promise<void> {
    const state = this.options.state
    const steer = state.pendingSteers.some((each) => each.id === id)
    let asked = false
    for (;;) {
      const client = await this.ensureOpen()
      try {
        if (asked) {
          await this.steersListed(client)
          const listed = state.pendingSteers.some((each) => each.id === id) ||
            state.queue.some((message) => message.id === id)
          if (!listed) {
            if (isWritten(state, id)) {
              throw refusedRemoval(steer)
            }
            return
          }
        }
        asked = true
        const outcome = state.pendingSteers.some((each) => each.id === id)
          ? await client.cancelPendingSteer(id)
          : await client.dequeueMessage(id)
        if (outcome === 'gone_on') {
          throw refusedRemoval(steer)
        }
        return
      } catch (error) {
        if (!(error instanceof ConversationSocketError)) {
          throw error
        }
      }
    }
  }

  /**
   * Sends a queued message now: the running turn ends at once without stopping
   * its commands, and the message runs next.
   */
  async sendQueuedNow(id: string): Promise<void> {
    const client = await this.ensureOpen()
    client.sendQueuedMessage(id)
  }

  /**
   * Delivers a pending steer now: the agent's running call returns at once with
   * its command still running, and the turn goes on with the steer.
   */
  async interruptPendingSteer(id: string): Promise<void> {
    const client = await this.ensureOpen()
    client.steerNow(id)
  }

  async abort(): Promise<void> {
    await (await this.ensureOpen()).abort()
  }

  /**
   * Names the children the page shows now to the open connection; a
   * connection still opening names them once it has, and one made later
   * before its `open`.
   */
  showSubagents(): void {
    this.client.value?.showSubagents(this.shownSubagents())
  }

  private shownSubagents(): readonly string[] {
    return this.options.shownSubagents?.() ?? []
  }

  async abortSubagents(): Promise<void> {
    const client = await this.ensureOpen()
    client.abortSubagents()
  }

  async abortSubagent(id: string): Promise<void> {
    const client = await this.ensureOpen()
    client.abortSubagent(id)
  }

  async abortTerminal(commandId: string): Promise<void> {
    const client = await this.ensureOpen()
    client.shellAbort(commandId)
  }

  async retry(): Promise<void> {
    this.options.state.lastError = null
    await untilEndedOrLost((await this.ensureOpen()).retry())
  }

  /**
   * The tail row says Requesting from the click until the server's next
   * `phase` event; a refused or lost request ends that wait as well. A
   * session busy running a turn already, as after another page's Resume,
   * runs the turn on: what the user asked for is there, and nothing failed. One
   * busy with other work, such as another page's Compact, may end without
   * running it, so that refusal is a failure.
   */
  async resume(): Promise<void> {
    const state = this.options.state
    state.lastError = null
    state.pendingAction = 'resume'
    try {
      await untilEndedOrLost((await this.ensureOpen()).resume())
    } catch (error) {
      this.settlePendingAction()
      // The session's phase arrives before its refusal, in order.
      if (errorCode(error) !== 'session_busy' || state.phase !== 'running') {
        throw error
      }
    }
  }

  private settlePendingAction(): void {
    this.options.state.pendingAction = null
  }

  /**
   * Compacts the context of `node`, a live subagent, or the root's when
   * absent. A subagent that ended meanwhile has no context left to compact,
   * which is no failure.
   */
  async compact(node?: string): Promise<void> {
    try {
      await untilEndedOrLost((await this.ensureOpen()).compact(node))
    } catch (error) {
      if (node === undefined || errorCode(error) !== 'agent_ended') {
        throw error
      }
    }
  }

  async connect(): Promise<void> {
    await this.ensureOpen()
  }

  /**
   * Lets the connection go, and opens none until `connect`, keeping all the
   * conversation shows, as an archived conversation does, whose socket the
   * backend closes. A connection the runtime was trying to bring back stops
   * being tried: the transcript it holds is what it shows.
   */
  suspend(): void {
    this.releaseConnection()
    // A conversation that opens no session stops at the first step of its
    // opening.
    const load = this.options.state.load
    if (load === 'reconnecting' || load === 'connecting' || load === 'opening') {
      this.options.state.load = 'ready'
    }
  }

  dispose(): void {
    this.disposed = true
    this.releaseConnection()
  }

  private releaseConnection(): void {
    this.settlePendingAction()
    this.unsubscribe?.()
    this.unsubscribe = null
    const controller = this.controller
    this.controller = null
    controller?.abort()
    this.client.value?.disconnect()
    this.client.value = null
    this.opening = null
  }

  private async ensureOpen(): Promise<ConversationClient> {
    // A request made as the page returns waits for the return's check, which
    // replaces a broken socket before anything is sent on it.
    const check = pendingReturnCheck()
    if (check !== null) {
      await check
    }
    return this.ensureOpenNow()
  }

  private ensureOpenNow(): Promise<ConversationClient> {
    if (this.disposed) {
      return Promise.reject(new Error('Conversation view was disposed'))
    }
    if (this.client.value) {
      return Promise.resolve(this.client.value)
    }
    return this.opening ?? this.startOpening(false)
  }

  /**
   * Starts the one opening, which the actions taken meanwhile wait for;
   * after a lost connection, it waits before its first attempt.
   */
  private startOpening(afterLoss: boolean): Promise<ConversationClient> {
    // Every first session of the conversation is an opening, which shows the
    // transcript with its agents until the session opens: a new
    // conversation's first send, one restored from the archive, a Retry
    // after a failed opening.
    if (!this.opened) {
      this.options.state.load = 'opening'
    }
    const controller = new AbortController()
    this.controller = controller
    const opening = this.openSession(controller, afterLoss)
    this.opening = opening
    return opening
  }

  /** One opening: attempts until the session opens, waiting before each attempt after a connection lost or not made. */
  private async openSession(controller: AbortController, afterLoss: boolean): Promise<ConversationClient> {
    for (let wait = afterLoss; ; wait = true) {
      if (wait) {
        // A first opening that cannot connect is still an opening: it says
        // that it connects and shows nothing of a session it never had.
        this.options.state.load = this.opened ? 'reconnecting' : 'connecting'
        this.connectionFailures += 1
        await this.pause(controller.signal)
      }
      try {
        const client = await this.openOnce(controller)
        this.connectionFailures = 0
        return client
      } catch (error) {
        if (this.controller !== controller) {
          // Released meanwhile: disposed, or a newer opening took over.
          throw error
        }
        if (!(error instanceof ConversationSocketError)) {
          this.client.value = null
          this.opening = null
          // A record that holds no model opens no session until one is
          // chosen: the opening stops at its first step, its transcript
          // shown, as one with no session does; it did not fail.
          if (errorCode(error) === 'model_not_selected') {
            this.options.state.load = 'ready'
          } else {
            this.options.state.load = 'failed'
            this.options.state.lastError = shownError(error)
          }
          throw error
        }
      }
    }
  }

  private async openOnce(controller: AbortController): Promise<ConversationClient> {
    let client: ConversationClient | null = null
    let unsubscribe: (() => void) | null = null
    // Each attempt has its own deadline; the opening's controller ends them all.
    const attempt = new AbortController()
    const abortAttempt = () => attempt.abort(controller.signal.reason)
    controller.signal.addEventListener('abort', abortAttempt, { once: true })
    const timeout = setTimeout(
      () => attempt.abort(new ConversationSocketError('The conversation did not open in time')),
      OPEN_TIMEOUT_MS,
    )
    // A deadline or a release while the session opens ends the connection, which ends the wait.
    const disconnect = () => client?.disconnect(asError(attempt.signal.reason))
    attempt.signal.addEventListener('abort', disconnect, { once: true })
    // The handshake's frames, held until the session has opened, so what
    // they show joins the conversation in one change with the session open,
    // and an attempt that fails shows none of them.
    let handshake: ClientSessionEvent[] | null = []
    try {
      client = await this.options.connect(attempt.signal)
      attempt.signal.throwIfAborted()
      const opened = client
      unsubscribe = client.subscribe((event) => {
        if (event.type === 'pending_steers') {
          this.listedSteers.add(opened)
        }
        if (this.controller !== controller) {
          return
        }
        if (this.client.value !== opened) {
          // Until the session opens, the connection's end is this attempt's:
          // `open` fails with it, and the opening decides what follows.
          if (event.type !== 'closed' && event.type !== 'disconnected') {
            handshake?.push(event)
          }
          return
        }
        this.applyEvent(event)
        triggerRef(this.client)
      })
      // The children the page shows go first, so the open's handshake
      // brings their transcripts and no other child's.
      const named = this.shownSubagents()
      if (named.length > 0) {
        client.showSubagents(named)
      }
      await client.open(() => heldEdge(this.options.state.history))
      attempt.signal.throwIfAborted()
      this.client.value = client
      this.unsubscribe = unsubscribe
      const shown = this.shownSubagents()
      if (shown.length !== named.length || shown.some((id) => !named.includes(id))) {
        client.showSubagents(shown)
      }
      // A child the page stopped showing while the handshake came, whose
      // transcript the page let go of, takes none of what it brought; a side
      // question's, which no page names, goes on.
      const dropped = named.filter((id) => !shown.includes(id))
      // Applied once, then let go: the subscription lives as long as the
      // connection, and the frames it held must not.
      const held = handshake
      handshake = null
      for (const event of held) {
        const childTranscript = event.type === 'subagent_transcript_reset' || event.type === 'subagent_transcript_patch'
        if (childTranscript && dropped.includes(event.subagentId)) {
          continue
        }
        this.applyEvent(event)
      }
      this.opened = true
      this.options.state.load = 'ready'
      // A reopened session is healthy; the failure that preceded it is over.
      this.options.state.lastError = null
      return client
    } catch (error) {
      unsubscribe?.()
      client?.disconnect()
      throw error
    } finally {
      clearTimeout(timeout)
      controller.signal.removeEventListener('abort', abortAttempt)
      attempt.signal.removeEventListener('abort', disconnect)
    }
  }

  /** Waits before the next attempt; releasing the connection ends the wait. */
  private pause(signal: AbortSignal): Promise<void> {
    return new Promise((resolve, reject) => {
      const onAbort = () => {
        wait.cancel()
        reject(signal.reason ?? new Error('The connection attempt was released'))
      }
      const wait = waitToReconnect(this.connectionFailures, () => {
        signal.removeEventListener('abort', onAbort)
        resolve()
      })
      signal.addEventListener('abort', onAbort, { once: true })
    })
  }

  private applyEvent(event: ClientSessionEvent): void {
    const state = this.options.state
    switch (event.type) {
      case 'transcript_reset':
        state.history = resetTranscript(state.history, event, event.asked)
        state.failures = { ...state.failures, ...event.failures }
        break
      case 'transcript_patch':
        state.history = applyTranscriptPatches(state.history, event.patches)
        state.failures = { ...state.failures, ...event.failures }
        break
      case 'phase':
        // A subagent's phase belongs to its record, which the host keeps.
        if (event.subagentId !== undefined) {
          break
        }
        // A failure the session reported is over once the session starts
        // another action, whichever tab started it; a tab that did not
        // start it would otherwise keep showing it.
        if (state.phase === 'idle' && event.phase !== 'idle') {
          state.lastError = null
        }
        state.phase = event.phase
        this.settlePendingAction()
        break
      case 'queue':
        state.queue = event.queue
        break
      case 'context_usage':
        // A subagent's usage belongs to its record, which the host keeps;
        // the root's node is the conversation's id.
        if (event.node === state.id) {
          state.contextUsage = event.usage
        }
        break
      case 'pending_steers':
        state.pendingSteers = event.pendingSteers
        break
      case 'pending_calls':
        // A subagent's calls belong to its record, which the host keeps.
        if (event.subagentId === undefined) {
          state.pendingCalls = event.pendingCalls
        }
        break
      case 'waiting':
        // So does a subagent's wait.
        if (event.subagentId === undefined) {
          state.waitingSince = event.since
        }
        break
      case 'error': {
        // Worded from its code as the transcript's record of a failed turn
        // is.
        const code = event.code
        state.lastError = {
          get message() {
            return failureHeadline(code, null, new Date())
          },
          requestId: event.requestId ?? null,
        }
        break
      }
      case 'rejected':
        // A saved message another page sent first is the caller's to tell.
        if (event.command !== 'send_saved') {
          state.lastError = shownError(new RefusedError(event))
        }
        this.settlePendingAction()
        break
      // A view never sends `close`, so a `closed` means another client
      // disposed the tree: the view opens it again, as after a lost
      // connection, and the backend restores it.
      case 'closed':
      case 'disconnected':
        this.releaseConnection()
        if (!this.disposed) {
          // The opening keeps trying on its own, and a refused open shows
          // through `load` and `lastError`; a release ends it with the view.
          this.startOpening(true).catch(() => {})
        }
        break
    }
    this.options.onEvent?.(event)
  }
}
