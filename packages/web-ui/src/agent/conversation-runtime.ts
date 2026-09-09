import type {
  AgentClient,
  ClientSessionEvent,
  ProviderSelection,
} from '@demicodes/agent/client'
import type { UserContentBlock } from '@demicodes/core'
import type { ConversationState } from './types'
import { createPendingSteerMessage } from './pending-steers'

export type RuntimeState = Pick<
  ConversationState,
  | 'id'
  | 'cwd'
  | 'blocks'
  | 'phase'
  | 'queue'
  | 'pendingSteers'
  | 'model'
  | 'lastError'
  | 'load'
>

export interface ConversationRuntimeOptions {
  state: RuntimeState
  connect: (signal: AbortSignal) => Promise<AgentClient>
  prepareModel: () => Promise<ProviderSelection>
  onEvent?: (event: ClientSessionEvent) => void
}

/** Owns one reconnectable client. Disposing a view leaves its server task alive. */
export class ConversationRuntime {
  private readonly options: ConversationRuntimeOptions
  private client: AgentClient | null = null
  private opening: Promise<AgentClient> | null = null
  private controller: AbortController | null = null
  private unsubscribe: (() => void) | null = null
  private retryTimer: ReturnType<typeof setTimeout> | null = null
  private disposed = false

  constructor(options: ConversationRuntimeOptions) {
    this.options = options
  }

  get connected(): boolean {
    return this.client !== null
  }

  async send(content: UserContentBlock[]): Promise<void> {
    this.options.state.lastError = null
    await (await this.ensureOpen()).send(content)
  }

  async submit(content: UserContentBlock[]): Promise<void> {
    this.options.state.lastError = null
    await (await this.ensureOpen()).submit(content)
  }

  async dequeueMessage(id: string): Promise<void> {
    const client = await this.ensureOpen()
    client.dequeueMessage(id)
  }

  async sendQueuedMessage(id: string): Promise<void> {
    const client = await this.ensureOpen()
    client.sendQueuedMessage(id)
  }

  async steerQueuedMessage(id: string): Promise<void> {
    await (await this.ensureOpen()).steerQueuedMessage(id)
  }

  async steer(content: UserContentBlock[]): Promise<void> {
    await (await this.ensureOpen()).steer(content)
  }

  async deletePendingSteer(id: string): Promise<void> {
    const client = await this.ensureOpen()
    client.cancelPendingSteer(id)
  }

  async interruptPendingSteer(id: string): Promise<void> {
    const pending = this.options.state.pendingSteers.find((item) => item.id === id)
    if (!pending) {
      return
    }
    await this.deletePendingSteer(id)
    await this.abort()
    await this.send(pending.content)
  }

  async setModel(): Promise<void> {
    const client = this.client
    const controller = this.controller
    if (!client || !controller) {
      return
    }
    const provider = await this.options.prepareModel()
    controller.signal.throwIfAborted()
    client.setProvider(provider)
  }

  async abort(): Promise<void> {
    await (await this.ensureOpen()).abort()
  }

  async abortSubagents(): Promise<void> {
    const client = await this.ensureOpen()
    client.abortSubagents()
  }

  async retry(): Promise<void> {
    this.options.state.lastError = null
    await (await this.ensureOpen()).retry()
  }

  async resume(): Promise<void> {
    this.options.state.lastError = null
    await (await this.ensureOpen()).resume()
  }

  async compact(): Promise<void> {
    await (await this.ensureOpen()).compact()
  }

  async connect(): Promise<void> {
    await this.ensureOpen()
  }

  async reconnect(): Promise<void> {
    this.releaseConnection()
    this.options.state.load = this.options.state.blocks.length
      ? 'reconnecting'
      : 'loading'
    await this.connect()
  }

  dispose(): void {
    this.disposed = true
    this.releaseConnection()
  }

  private releaseConnection(): void {
    if (this.retryTimer !== null) {
      clearTimeout(this.retryTimer)
    }
    this.retryTimer = null
    this.unsubscribe?.()
    this.unsubscribe = null
    const controller = this.controller
    this.controller = null
    controller?.abort()
    this.client?.disconnect()
    this.client = null
    this.opening = null
  }

  private ensureOpen(): Promise<AgentClient> {
    if (this.disposed) {
      return Promise.reject(new Error('Conversation view was disposed'))
    }
    if (this.client) {
      return Promise.resolve(this.client)
    }
    if (!this.opening) {
      const controller = new AbortController()
      this.controller = controller
      this.opening = this.openSession(controller)
    }
    return this.opening
  }

  private async openSession(controller: AbortController): Promise<AgentClient> {
    let client: AgentClient | null = null
    let unsubscribe: (() => void) | null = null
    const timeout = setTimeout(
      () => controller.abort(new Error('The conversation did not open in time')),
      30_000,
    )
    try {
      const provider = await this.options.prepareModel()
      controller.signal.throwIfAborted()
      client = await this.options.connect(controller.signal)
      controller.signal.throwIfAborted()
      unsubscribe = client.subscribe((event) => {
        if (this.controller === controller) {
          this.applyEvent(event)
        }
      })
      await client.open(provider, this.options.state.cwd, this.options.state.id)
      controller.signal.throwIfAborted()
      this.client = client
      this.unsubscribe = unsubscribe
      this.options.state.load = 'ready'
      return client
    } catch (error) {
      unsubscribe?.()
      client?.disconnect()
      if (this.controller === controller) {
        this.client = null
        this.opening = null
        this.options.state.load = 'failed'
        this.options.state.lastError =
          error instanceof Error ? error.message : String(error)
      }
      throw error
    } finally {
      clearTimeout(timeout)
    }
  }

  private applyEvent(event: ClientSessionEvent): void {
    const state = this.options.state
    switch (event.type) {
      case 'transcript_reset':
      case 'transcript_patch':
        state.blocks = event.blocks
        break
      case 'phase':
        state.phase = event.phase
        break
      case 'queue':
        state.queue = event.queue
        break
      case 'pending_steers':
        state.pendingSteers = event.pendingSteers.map((pending) =>
          createPendingSteerMessage(pending.id, pending.content, state.blocks),
        )
        break
      case 'error':
        state.lastError = event.message
        break
      case 'rejected':
        state.lastError = event.reason
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
          state.load = state.blocks.length ? 'reconnecting' : 'loading'
          this.retryTimer = setTimeout(() => {
            this.retryTimer = null
            // openSession records a failed reconnect for the shared Retry UI.
            void this.connect().catch(() => {})
          }, 1000)
        }
        break
    }
    this.options.onEvent?.(event)
  }
}
