import { shallowRef, triggerRef } from 'vue'
import { SessionError, SteerRejectedError, type AgentClient, type ClientSessionEvent } from '@demicodes/agent-client'
import { asError } from '@demicodes/utils'
import type { ClientContent, EditRequest, ModelSelection, TranscriptVersion } from '@demicodes/protocol'
import { AgentSocketError } from '../transport/agent-socket'
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
  | 'model'
  | 'lastError'
  | 'load'
  | 'pendingAction'
  | 'failures'
>

export interface ConversationRuntimeOptions {
  state: RuntimeState
  connect: (signal: AbortSignal) => Promise<AgentClient>
  prepareModel: () => Promise<ModelSelection>
  onEvent?: (event: ClientSessionEvent) => void
  /** The wait before the first reconnect attempt; every later one doubles it, up to `maxMs`. */
  reconnect?: { baseMs: number; maxMs: number }
}

const DEFAULT_RECONNECT = { baseMs: 1000, maxMs: 15_000 }
const OPEN_TIMEOUT_MS = 30_000

/**
 * Owns one reconnectable client. Disposing a view leaves its server task alive.
 *
 * A connection that cannot be made or is lost is never a failure the reader
 * is told about: on a weak network it comes and goes, so the runtime keeps
 * `load` at `reconnecting` (the transcript's tail row says Connecting, the
 * composer stays) and retries with backoff until the socket is back or the
 * view is disposed. An action taken meanwhile waits for the connection.
 * Only the session refusing to open is a failure, told once through `load`
 * `failed` and `lastError`.
 */
export class ConversationRuntime {
  private readonly options: ConversationRuntimeOptions
  private readonly client = shallowRef<AgentClient | null>(null)
  private opening: Promise<AgentClient> | null = null
  private controller: AbortController | null = null
  private unsubscribe: (() => void) | null = null
  private retryTimer: ReturnType<typeof setTimeout> | null = null
  private disposed = false

  constructor(options: ConversationRuntimeOptions) {
    this.options = options
  }

  get connected(): boolean {
    return this.client.value !== null
  }

  async submit(content: ClientContent[], messageId?: string): Promise<void> {
    this.options.state.lastError = null
    const client = await this.ensureOpen()
    if (messageId && hasAcceptedSubmission(this.options.state, messageId)) {
      return
    }
    await client.submit(content, messageId)
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
   * Sends a queued message now (`product.md` § Conversations and projects):
   * while a turn runs, it becomes a steer of that turn; otherwise it moves to
   * the front of the queue and runs next. A turn that refuses the steer, as
   * one that is ending does, leaves the message queued, so it moves to the
   * front instead and the refusal is no error.
   */
  async sendQueuedNow(id: string): Promise<void> {
    const client = await this.ensureOpen()
    if (this.options.state.phase !== 'idle') {
      try {
        await client.steerQueuedMessage(id)
        return
      } catch (error) {
        if (!(error instanceof SteerRejectedError)) {
          throw error
        }
      }
    }
    client.sendQueuedMessage(id)
  }

  async deletePendingSteer(id: string): Promise<void> {
    const client = await this.ensureOpen()
    client.cancelPendingSteer(id)
  }

  /**
   * Delivers a pending steer now: Stop writes the steers still pending into
   * the stopped turn, and Continue goes on from there with it
   * (`product.md` § Conversations and projects).
   */
  async interruptPendingSteer(id: string): Promise<void> {
    if (!this.options.state.pendingSteers.some((item) => item.id === id)) {
      return
    }
    await this.abort()
    await this.resume()
  }

  async setModel(): Promise<void> {
    const client = this.client.value
    const controller = this.controller
    if (!client || !controller) {
      return
    }
    const model = await this.options.prepareModel()
    controller.signal.throwIfAborted()
    client.setProvider(model)
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
    if (this.retryTimer !== null) {
      clearTimeout(this.retryTimer)
    }
    this.retryTimer = null
    this.unsubscribe?.()
    this.unsubscribe = null
    const controller = this.controller
    this.controller = null
    controller?.abort()
    this.client.value?.disconnect()
    this.client.value = null
    this.opening = null
  }

  private ensureOpen(): Promise<AgentClient> {
    if (this.disposed) {
      return Promise.reject(new Error('Conversation view was disposed'))
    }
    if (this.client.value) {
      return Promise.resolve(this.client.value)
    }
    if (!this.opening) {
      const controller = new AbortController()
      this.controller = controller
      this.opening = this.openSession(controller)
    }
    return this.opening
  }

  /** One opening: attempts until the session opens, backing off after each lost connection. */
  private async openSession(controller: AbortController): Promise<AgentClient> {
    const reconnect = this.options.reconnect ?? DEFAULT_RECONNECT
    let delay = reconnect.baseMs
    for (;;) {
      try {
        return await this.openOnce(controller)
      } catch (error) {
        if (this.controller !== controller) {
          // Released meanwhile: disposed, or a newer opening took over.
          throw error
        }
        if (!(error instanceof AgentSocketError)) {
          this.client.value = null
          this.opening = null
          this.options.state.load = 'failed'
          this.options.state.lastError =
            error instanceof Error ? error.message : String(error)
          throw error
        }
        this.options.state.load = 'reconnecting'
        await this.pause(delay, controller.signal)
        delay = Math.min(delay * 2, reconnect.maxMs)
      }
    }
  }

  private async openOnce(controller: AbortController): Promise<AgentClient> {
    let client: AgentClient | null = null
    let unsubscribe: (() => void) | null = null
    // Each attempt has its own deadline; the opening's controller ends them all.
    const attempt = new AbortController()
    const abortAttempt = () => attempt.abort(controller.signal.reason)
    controller.signal.addEventListener('abort', abortAttempt, { once: true })
    const timeout = setTimeout(
      () => attempt.abort(new AgentSocketError('The conversation did not open in time')),
      OPEN_TIMEOUT_MS,
    )
    // A deadline or a release while the session opens ends the connection, which ends the wait.
    const disconnect = () => client?.disconnect(asError(attempt.signal.reason))
    attempt.signal.addEventListener('abort', disconnect, { once: true })
    try {
      const model = await this.options.prepareModel()
      attempt.signal.throwIfAborted()
      client = await this.options.connect(attempt.signal)
      attempt.signal.throwIfAborted()
      unsubscribe = client.subscribe((event) => {
        if (this.controller === controller) {
          this.applyEvent(event)
          triggerRef(this.client)
        }
      })
      await client.open(model)
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
  private pause(ms: number, signal: AbortSignal): Promise<void> {
    return new Promise((resolve, reject) => {
      const onAbort = () => {
        clearTimeout(timer)
        this.retryTimer = null
        reject(signal.reason ?? new Error('The connection attempt was released'))
      }
      const timer = setTimeout(() => {
        signal.removeEventListener('abort', onAbort)
        this.retryTimer = null
        resolve()
      }, ms)
      this.retryTimer = timer
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
        state.phase = event.phase
        this.settlePendingAction()
        break
      case 'queue':
        state.queue = event.queue
        break
      case 'pending_steers':
        state.pendingSteers = event.pendingSteers
        break
      case 'error':
        state.lastError = event.message
        break
      case 'rejected':
        state.lastError = event.reason
        this.settlePendingAction()
        break
      case 'closed':
        // Another view can take over the server attachment. Reconnect only on
        // a user action; automatic reconnect here would make the views fight.
        this.releaseConnection()
        state.load = 'ready'
        break
      case 'disconnected':
        this.releaseConnection()
        if (!this.disposed) {
          state.load = 'reconnecting'
          this.retryTimer = setTimeout(() => {
            this.retryTimer = null
            // The opening keeps trying on its own; only a refused open rejects here.
            void this.connect().catch(() => {})
          }, this.options.reconnect?.baseMs ?? DEFAULT_RECONNECT.baseMs)
        }
        break
    }
    this.options.onEvent?.(event)
  }
}
