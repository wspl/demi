import { shallowRef, triggerRef } from 'vue'
import { SessionError, type ConversationClient, type ClientSessionEvent } from '@demicodes/conversation-client'
import { asError, createId } from '@demicodes/utils'
import type { ClientContent, EditRequest, TranscriptVersion } from '@demicodes/protocol'
import { ConversationSocketError } from '../transport/conversation-socket'
import { waitToReconnect } from '../transport/liveness'
import { hasAcceptedSubmission } from './submission'
import type { ConversationState } from './types'

/**
 * A rejected action whose failure the session reported as an `error` frame:
 * a failed turn is a record in the transcript, and a refused frame or a
 * failed save is the session's `lastError`, so the caller shows nothing else.
 */
export function isRecordedTurnFailure(error: unknown): boolean {
  return error instanceof SessionError
}

export type RuntimeState = Pick<
  ConversationState,
  | 'blocks'
  | 'phase'
  | 'queue'
  | 'pendingSteers'
  | 'pendingCalls'
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
}

const OPEN_TIMEOUT_MS = 30_000

/**
 * Owns one reconnectable client. Disposing a view leaves its server task alive.
 *
 * A connection that cannot be made or is lost, before the session answered
 * `open` or after, or whose tree another client disposed, is never a
 * failure the reader is told about: on a weak network it comes and goes, so
 * the runtime keeps `load` at `reconnecting` (the transcript's tail row says
 * Connecting, the composer stays) and tries again after the page's reconnect
 * waits (`web-application.md` § Liveness and reconnection) until the socket
 * is back or the view is disposed. An action taken meanwhile waits for the
 * connection, without cutting a wait short.
 * Only the session refusing to open is a failure, told once through `load`
 * `failed` and `lastError`.
 */
export class ConversationRuntime {
  private readonly options: ConversationRuntimeOptions
  private readonly client = shallowRef<ConversationClient | null>(null)
  private opening: Promise<ConversationClient> | null = null
  private controller: AbortController | null = null
  private unsubscribe: (() => void) | null = null
  /**
   * The clients whose session has listed its pending steers, the last of the
   * frames that show the conversation as it opens
   * (`runtime.md` § Pending steers).
   */
  private readonly listedSteers = new WeakSet<ConversationClient>()
  /** Consecutive connections lost or not made since the session last opened. */
  private connectionFailures = 0
  private disposed = false

  constructor(options: ConversationRuntimeOptions) {
    this.options = options
  }

  get connected(): boolean {
    return this.client.value !== null
  }

  /**
   * Sends a message; resolves once the session holds it, in the transcript
   * or the queue. A connection lost before the session confirmed it fails
   * nothing: the send waits for the connection to come back, then sends the
   * message again with its id, which the session takes only once
   * (`runtime.md` § Messages and the queue), unless what the reopened
   * conversation shows already holds it.
   */
  async submit(content: ClientContent[], messageId: string = createId()): Promise<void> {
    this.options.state.lastError = null
    for (;;) {
      const client = await this.ensureOpen()
      if (hasAcceptedSubmission(this.options.state, messageId)) {
        return
      }
      try {
        await client.submit(content, messageId)
        return
      } catch (error) {
        if (!(error instanceof ConversationSocketError)) {
          throw error
        }
      }
    }
  }

  /**
   * Steers the running turn with a message (`product.md` § Steer or queue);
   * resolves once the session holds it as a pending steer. A steer the
   * session refuses, because its turn is ending or nothing runs, is sent as
   * a message instead, with the same id and no error. A connection lost
   * before the session answered fails nothing: once the conversation is open
   * again and has listed its pending steers, a steer it neither lists nor
   * holds in the transcript is sent as a message, so it is never lost and
   * never delivered twice.
   */
  async steer(content: ClientContent[], id: string = createId()): Promise<void> {
    this.options.state.lastError = null
    const client = await this.ensureOpen()
    try {
      await client.steer(content, id)
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
    await this.submit(content, id)
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

  async editAndSend(request: EditRequest): Promise<void> {
    const client = await this.ensureOpen()
    const replacesVisibleTarget = client.transcript().blocks.some(
      (block) => block.id === request.targetBlockId,
    )
    let receivedError = false
    const unsubscribe = client.subscribe((event) => {
      if (event.type === 'error') {
        receivedError = true
      }
    })
    try {
      await client.editAndSend(request)
      // Reconciliation can confirm an earlier edit after its generation failed.
      // That receipt must not erase the current turn's recovery action.
      if (replacesVisibleTarget && !receivedError) {
        this.options.state.lastError = null
      }
    } finally {
      unsubscribe()
    }
  }

  async dequeueMessage(id: string): Promise<void> {
    const client = await this.ensureOpen()
    client.dequeueMessage(id)
  }

  /**
   * Sends a queued message now (`product.md` § Steer or queue): the running
   * turn ends at once without stopping its commands, and the message runs
   * next.
   */
  async sendQueuedNow(id: string): Promise<void> {
    const client = await this.ensureOpen()
    client.sendQueuedMessage(id)
  }

  async deletePendingSteer(id: string): Promise<void> {
    const client = await this.ensureOpen()
    client.cancelPendingSteer(id)
  }

  /**
   * Delivers a pending steer now (`product.md` § Steer or queue): the agent's
   * running call returns at once with its command still running, and the
   * turn goes on with the steer.
   */
  async interruptPendingSteer(id: string): Promise<void> {
    const client = await this.ensureOpen()
    client.steerNow(id)
  }

  async abort(): Promise<void> {
    await (await this.ensureOpen()).abort()
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
    await (await this.ensureOpen()).retry()
  }

  /**
   * The tail row says Requesting from the click until the server's next
   * `phase` event; a refused or lost request ends that wait as well.
   */
  async resume(): Promise<void> {
    const state = this.options.state
    state.lastError = null
    state.pendingAction = 'resume'
    try {
      await (await this.ensureOpen()).resume()
    } catch (error) {
      this.settlePendingAction()
      throw error
    }
  }

  private settlePendingAction(): void {
    this.options.state.pendingAction = null
  }

  async compact(): Promise<void> {
    await (await this.ensureOpen()).compact()
  }

  async connect(): Promise<void> {
    await this.ensureOpen()
  }

  async reconnect(): Promise<void> {
    this.releaseConnection()
    this.options.state.load = 'reconnecting'
    await this.connect()
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

  private ensureOpen(): Promise<ConversationClient> {
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
        this.options.state.load = 'reconnecting'
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
          this.options.state.load = 'failed'
          this.options.state.lastError =
            error instanceof Error ? error.message : String(error)
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
        // Until the session opens, the connection's end is this attempt's:
        // `open` fails with it, and the opening decides what follows.
        const ended = event.type === 'closed' || event.type === 'disconnected'
        if (ended && this.client.value !== client) {
          return
        }
        this.applyEvent(event)
        triggerRef(this.client)
      })
      await client.open()
      attempt.signal.throwIfAborted()
      this.client.value = client
      this.unsubscribe = unsubscribe
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
      case 'transcript_patch':
        state.blocks = event.blocks
        state.failures = event.failures
        break
      case 'phase':
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
        state.contextUsage = event.usage
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
      case 'error':
        state.lastError = event.message
        break
      case 'rejected':
        state.lastError = event.reason
        this.settlePendingAction()
        break
      // A view never sends `close`, so a `closed` means another client
      // disposed the tree: the view opens it again, as after a lost
      // connection, and the backend restores it (`runtime.md` § Connections
      // and the live tree).
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
