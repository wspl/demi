import { computed, inject, provide, reactive, ref, type ComputedRef, type InjectionKey } from 'vue'
import type { UserContentBlock } from '@demicodes/core'
import type { ConversationSummary } from '@demicodes/agent'
import type { ControlApi, ModelInfo, ProviderInfo } from '../transport/protocol'
import { agentSocketUrl, connectAgentClient } from '../transport/agent-socket'
import { ConversationRuntime } from './conversation-runtime'
import type { ConversationState, ModelIntent } from './types'

export interface AgentWorkspaceParams {
  baseUrl: string
  control: ControlApi
  cwd: string
  idFactory?: () => string
}

interface PersistedWorkspace {
  order: string[]
  activeId: string | null
  conversations: { id: string; title: string; createdAt: string; model: ModelIntent }[]
}

function workspaceStorage(): Storage | null {
  try {
    return typeof localStorage !== 'undefined' ? localStorage : null
  } catch {
    return null
  }
}

/**
 * Client-side workspace store. Owns the open conversations (tabs), their
 * reactive state, and the AgentClient runtimes. Replaces agent-gui's
 * server-synced `rpc.agent`/`rpc.project` state.
 */
export class AgentWorkspace {
  readonly sessions = reactive<Record<string, ConversationState>>({})
  readonly order = ref<string[]>([])
  readonly activeId = ref<string | null>(null)
  readonly providers = ref<ProviderInfo[]>([])
  readonly models = reactive<Record<string, ModelInfo[]>>({})
  // All persisted conversations for this cwd, from the server (the history list,
  // independent of which tabs are open or of localStorage).
  readonly serverConversations = ref<ConversationSummary[]>([])

  readonly tabs: ComputedRef<ConversationState[]> = computed(() =>
    this.order.value.map((id) => this.sessions[id]).filter((state): state is ConversationState => !!state),
  )
  readonly activeSession: ComputedRef<ConversationState | null> = computed(() =>
    this.activeId.value ? this.sessions[this.activeId.value] ?? null : null,
  )

  private readonly runtimes = new Map<string, ConversationRuntime>()
  private readonly baseUrl: string
  private readonly control: ControlApi
  private readonly cwd: string
  private readonly idFactory: () => string
  private defaultModel: ModelIntent | null = null
  private titleCounter = 0

  private readonly storageKey: string

  constructor(params: AgentWorkspaceParams) {
    this.baseUrl = params.baseUrl
    this.control = params.control
    this.cwd = params.cwd
    this.idFactory = params.idFactory ?? (() => globalThis.crypto.randomUUID())
    this.storageKey = `demi.conversations.${params.cwd}`
  }

  async init(): Promise<void> {
    await this.loadCatalog()
    // Restore previously open conversations (their transcripts come back from
    // the server on connect, keyed by the conversation id). Fall back to a
    // fresh one when nothing is persisted.
    const restored = this.restorePersisted()
    // The server owns the full conversation list (survives a cleared browser).
    this.serverConversations.value = await this.loadServerConversations()
    if (!restored) {
      // Nothing remembered locally — recover the most recent conversation from
      // the server, or start fresh if there is none.
      const recent = this.serverConversations.value[0]
      if (recent) this.openConversation(recent)
      else this.createConversation()
    }
    // Connect the visible conversation so a restored transcript loads on open.
    this.connectActive()
  }

  /** Re-fetch the server conversation list (e.g. when the history menu opens). */
  async refreshServerConversations(): Promise<void> {
    this.serverConversations.value = await this.loadServerConversations()
  }

  /** Open a conversation by id (an existing tab, or one from the history list). */
  openConversation(summary: { id: string; title?: string }): void {
    if (!this.sessions[summary.id]) {
      this.materializeConversation({
        id: summary.id,
        title: summary.title || this.nextTitle(),
        createdAt: new Date().toISOString(),
        model: this.defaultModel ?? this.fallbackModel(),
      })
      this.persist()
    }
    this.setActive(summary.id)
    this.connectActive()
  }

  private async loadServerConversations(): Promise<ConversationSummary[]> {
    try {
      const client = await connectAgentClient(agentSocketUrl(this.baseUrl, this.cwd))
      try {
        return await client.listConversations(this.cwd)
      } finally {
        await client.close().catch(() => {})
      }
    } catch {
      return this.serverConversations.value
    }
  }

  private connectActive(): void {
    const id = this.activeId.value
    if (id) void this.runtimes.get(id)?.connect().catch(() => {})
  }

  async loadCatalog(): Promise<void> {
    const providers = await this.control.listProviders()
    this.providers.value = providers
    for (const provider of providers) {
      if (!provider.isAvailable) continue
      try {
        this.models[provider.id] = await this.control.listModels({ providerId: provider.id })
      } catch {
        this.models[provider.id] = []
      }
    }
    this.defaultModel = this.resolveDefaultModel()
  }

  createConversation(options: { afterId?: string; title?: string } = {}): string {
    const id = this.materializeConversation(
      {
        id: this.idFactory(),
        title: options.title ?? this.nextTitle(),
        createdAt: new Date().toISOString(),
        model: this.defaultModel ?? this.fallbackModel(),
      },
      options.afterId,
    )
    this.activeId.value = id
    this.persist()
    return id
  }

  // Create the reactive state + runtime for a conversation and slot it into the
  // tab order. Shared by new conversations and persistence restore.
  private materializeConversation(
    meta: { id: string; title: string; createdAt: string; model: ModelIntent },
    afterId?: string,
  ): string {
    const state = reactive<ConversationState>({
      id: meta.id,
      cwd: this.cwd,
      title: meta.title,
      createdAt: meta.createdAt,
      blocks: [],
      phase: 'idle',
      queue: [],
      pendingSteers: [],
      model: meta.model,
      draft: null,
      isResultSeen: true,
      hasContent: false,
      lastError: null,
    })
    this.sessions[meta.id] = state
    this.runtimes.set(meta.id, new ConversationRuntime(state, this.baseUrl, this.control))
    const index = afterId ? this.order.value.indexOf(afterId) + 1 : this.order.value.length
    this.order.value = [...this.order.value.slice(0, index), meta.id, ...this.order.value.slice(index)]
    return meta.id
  }

  async closeConversation(id: string): Promise<void> {
    this.order.value = this.order.value.filter((entry) => entry !== id)
    delete this.sessions[id]
    if (this.activeId.value === id) this.activeId.value = this.order.value[0] ?? null
    const runtime = this.runtimes.get(id)
    this.runtimes.delete(id)
    this.persist()
    await runtime?.dispose()
  }

  setActive(id: string): void {
    if (!this.sessions[id]) return
    this.activeId.value = id
    const state = this.sessions[id]
    if (state) state.isResultSeen = true
    this.connectActive()
    this.persist()
  }

  reorderTabs(ids: string[]): void {
    this.order.value = ids
    this.persist()
  }

  renameConversation(id: string, title: string): void {
    const state = this.sessions[id]
    if (state) state.title = title
    this.persist()
  }

  setModel(id: string, model: ModelIntent): void {
    const state = this.sessions[id]
    if (state) state.model = model
    // Push to an open session so the next turn uses the new model; no-op until it opens.
    void this.runtimes.get(id)?.setModel()
    this.persist()
  }

  send(id: string, content: UserContentBlock[]): Promise<void> {
    return this.runtime(id).send(content)
  }

  dequeueMessage(id: string, messageId: string): void {
    this.runtimes.get(id)?.dequeueMessage(messageId)
  }

  sendQueuedMessage(id: string, messageId: string): void {
    this.runtimes.get(id)?.sendQueuedMessage(messageId)
  }

  steerQueuedMessage(id: string, messageId: string): Promise<void> {
    return this.runtime(id).steerQueuedMessage(messageId)
  }

  clearMessageQueue(id: string): void {
    const messageIds = this.sessions[id]?.queue.map((message) => message.id) ?? []
    this.runtimes.get(id)?.clearMessageQueue(messageIds)
  }

  steer(id: string, content: UserContentBlock[]): Promise<void> {
    return this.runtime(id).steer(content)
  }

  deletePendingSteer(id: string, steerId: string): void {
    this.runtimes.get(id)?.deletePendingSteer(steerId)
  }

  abort(id: string): Promise<void> {
    return this.runtime(id).abort()
  }

  retry(id: string): Promise<void> {
    return this.runtime(id).retry()
  }

  resume(id: string): Promise<void> {
    return this.runtime(id).resume()
  }

  compact(id: string): Promise<void> {
    return this.runtime(id).compact()
  }

  async dispose(): Promise<void> {
    const runtimes = [...this.runtimes.values()]
    this.runtimes.clear()
    await Promise.all(runtimes.map((runtime) => runtime.dispose()))
  }

  private runtime(id: string): ConversationRuntime {
    const runtime = this.runtimes.get(id)
    if (!runtime) throw new Error(`No conversation runtime for ${id}`)
    return runtime
  }

  // Persist the conversation list (ids/titles/models/order) per cwd. Transcripts
  // are not stored here — they are restored from the server by conversation id.
  private persist(): void {
    const storage = workspaceStorage()
    if (!storage) return
    const conversations = this.order.value
      .map((id) => this.sessions[id])
      .filter((state): state is ConversationState => !!state)
      .map((state) => ({ id: state.id, title: state.title, createdAt: state.createdAt, model: state.model }))
    const payload: PersistedWorkspace = { order: this.order.value, activeId: this.activeId.value, conversations }
    try {
      storage.setItem(this.storageKey, JSON.stringify(payload))
    } catch {
      // Storage full or unavailable — persistence is best-effort.
    }
  }

  // Returns true if conversations were restored from storage.
  private restorePersisted(): boolean {
    const storage = workspaceStorage()
    if (!storage) return false
    let payload: PersistedWorkspace | null = null
    try {
      const raw = storage.getItem(this.storageKey)
      payload = raw ? (JSON.parse(raw) as PersistedWorkspace) : null
    } catch {
      payload = null
    }
    const conversations = payload?.conversations ?? []
    if (conversations.length === 0) return false
    const byId = new Map(conversations.map((conversation) => [conversation.id, conversation]))
    const order = (payload?.order ?? []).filter((id) => byId.has(id))
    for (const id of order) {
      const meta = byId.get(id)!
      this.materializeConversation({ id: meta.id, title: meta.title, createdAt: meta.createdAt, model: meta.model })
    }
    this.titleCounter = order.length
    this.activeId.value = payload?.activeId && byId.has(payload.activeId) ? payload.activeId : order[0] ?? null
    return order.length > 0
  }

  private resolveDefaultModel(): ModelIntent | null {
    for (const provider of this.providers.value) {
      const model = this.models[provider.id]?.[0]
      if (model) {
        return {
          providerId: provider.id,
          modelId: model.id,
          thinkingEffort: model.reasoning?.defaultEffort ?? null,
          serviceTierId: null,
        }
      }
    }
    return null
  }

  private fallbackModel(): ModelIntent {
    return { providerId: 'claude-code', modelId: 'default', thinkingEffort: null, serviceTierId: null }
  }

  private nextTitle(): string {
    this.titleCounter += 1
    return `Conversation ${this.titleCounter}`
  }
}

const AGENT_WORKSPACE_KEY: InjectionKey<AgentWorkspace> = Symbol('demi.agent-workspace')

export function provideAgentWorkspace(workspace: AgentWorkspace): void {
  provide(AGENT_WORKSPACE_KEY, workspace)
}

export function useAgentWorkspace(): AgentWorkspace {
  const workspace = inject(AGENT_WORKSPACE_KEY)
  if (!workspace) throw new Error('AgentWorkspace is not provided')
  return workspace
}
