import type { AgentClient, ClientSessionEvent } from '@demi/agent/client'
import type { UserContentBlock } from '@demi/core'
import { agentSocketUrl, connectAgentClient } from '../transport/agent-socket'
import type { ControlApi } from '../transport/protocol'
import type { ConversationState } from './types'

/**
 * Non-reactive owner of one conversation's AgentClient. Lazily connects the
 * per-session WebSocket on first action and writes incoming events into the
 * reactive ConversationState.
 */
export class ConversationRuntime {
  private client: AgentClient | null = null
  private opening: Promise<AgentClient> | null = null
  private unsubscribe: (() => void) | null = null

  constructor(
    private readonly state: ConversationState,
    private readonly baseUrl: string,
    private readonly control: ControlApi,
  ) {}

  async send(content: UserContentBlock[]): Promise<void> {
    const client = await this.ensureOpen()
    this.state.isResultSeen = true
    await client.send(content)
  }

  async steer(content: UserContentBlock[]): Promise<void> {
    const client = await this.ensureOpen()
    this.state.isResultSeen = true
    await client.steer(content)
  }

  /**
   * Pushes a model/provider switch to an already-open session so the next turn uses it. If the
   * session has not been opened yet, this is a no-op: openSession reads the latest state.model.
   */
  async setModel(): Promise<void> {
    if (!this.client) return
    const intent = this.state.model
    const providerConfig = await this.control.prepareSession({
      providerType: intent.providerType,
      modelId: intent.modelId,
      thinkingEffort: intent.thinkingEffort,
      serviceTierId: intent.serviceTierId,
    })
    this.client.setProvider(providerConfig)
  }

  async abort(): Promise<void> {
    await this.client?.abort()
  }

  async retry(): Promise<void> {
    await (await this.ensureOpen()).retry()
  }

  async resume(): Promise<void> {
    await (await this.ensureOpen()).resume()
  }

  async compact(): Promise<void> {
    await (await this.ensureOpen()).compact()
  }

  async dispose(): Promise<void> {
    this.unsubscribe?.()
    this.unsubscribe = null
    const client = this.client
    this.client = null
    this.opening = null
    if (client) await client.close().catch(() => {})
  }

  private ensureOpen(): Promise<AgentClient> {
    if (this.client) return Promise.resolve(this.client)
    this.opening ??= this.openSession()
    return this.opening
  }

  private async openSession(): Promise<AgentClient> {
    const client = await connectAgentClient(agentSocketUrl(this.baseUrl, this.state.cwd))
    this.unsubscribe = client.subscribe((event) => this.applyEvent(event))
    const intent = this.state.model
    const providerConfig = await this.control.prepareSession({
      providerType: intent.providerType,
      modelId: intent.modelId,
      thinkingEffort: intent.thinkingEffort,
      serviceTierId: intent.serviceTierId,
    })
    await client.open(providerConfig, this.state.cwd)
    this.client = client
    return client
  }

  private applyEvent(event: ClientSessionEvent): void {
    switch (event.type) {
      case 'transcript_snapshot':
      case 'transcript_patch':
        this.state.blocks = event.blocks
        this.state.hasContent = event.blocks.length > 0
        return
      case 'phase':
        this.state.phase = event.phase
        return
      case 'queue':
        this.state.queue = event.queue
        return
      case 'error':
        this.state.lastError = event.message
        return
      case 'closed':
        this.client = null
        this.opening = null
        return
    }
  }
}
