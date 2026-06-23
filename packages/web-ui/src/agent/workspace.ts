import { computed, inject, provide, reactive, ref, type ComputedRef, type InjectionKey } from 'vue'
import type { UserContentBlock } from '@demi/core'
import type { ControlApi, ModelInfo, ProviderInfo } from '../transport/protocol'
import { ConversationRuntime } from './conversation-runtime'
import type { ConversationState, ModelIntent } from './types'

export interface AgentWorkspaceParams {
  baseUrl: string
  control: ControlApi
  cwd: string
  idFactory?: () => string
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

  constructor(params: AgentWorkspaceParams) {
    this.baseUrl = params.baseUrl
    this.control = params.control
    this.cwd = params.cwd
    this.idFactory = params.idFactory ?? (() => globalThis.crypto.randomUUID())
  }

  async init(): Promise<void> {
    await this.loadCatalog()
    if (this.order.value.length === 0) this.createConversation()
  }

  async loadCatalog(): Promise<void> {
    const providers = await this.control.listProviders()
    this.providers.value = providers
    for (const provider of providers) {
      if (!provider.isAvailable) continue
      try {
        this.models[provider.type] = await this.control.listModels({ providerType: provider.type })
      } catch {
        this.models[provider.type] = []
      }
    }
    this.defaultModel = this.resolveDefaultModel()
  }

  createConversation(options: { afterId?: string; title?: string } = {}): string {
    const id = this.idFactory()
    const state = reactive<ConversationState>({
      id,
      cwd: this.cwd,
      title: options.title ?? this.nextTitle(),
      createdAt: new Date().toISOString(),
      blocks: [],
      phase: 'idle',
      queue: [],
      model: this.defaultModel ?? this.fallbackModel(),
      draft: null,
      isResultSeen: true,
      hasContent: false,
      lastError: null,
    })
    this.sessions[id] = state
    this.runtimes.set(id, new ConversationRuntime(state, this.baseUrl, this.control))
    const index = options.afterId ? this.order.value.indexOf(options.afterId) + 1 : this.order.value.length
    this.order.value = [...this.order.value.slice(0, index), id, ...this.order.value.slice(index)]
    this.activeId.value = id
    return id
  }

  async closeConversation(id: string): Promise<void> {
    this.order.value = this.order.value.filter((entry) => entry !== id)
    delete this.sessions[id]
    if (this.activeId.value === id) this.activeId.value = this.order.value[0] ?? null
    const runtime = this.runtimes.get(id)
    this.runtimes.delete(id)
    await runtime?.dispose()
  }

  setActive(id: string): void {
    if (!this.sessions[id]) return
    this.activeId.value = id
    const state = this.sessions[id]
    if (state) state.isResultSeen = true
  }

  reorderTabs(ids: string[]): void {
    this.order.value = ids
  }

  renameConversation(id: string, title: string): void {
    const state = this.sessions[id]
    if (state) state.title = title
  }

  setModel(id: string, model: ModelIntent): void {
    const state = this.sessions[id]
    if (state) state.model = model
    // Push to an open session so the next turn uses the new model; no-op until it opens.
    void this.runtimes.get(id)?.setModel()
  }

  send(id: string, content: UserContentBlock[]): Promise<void> {
    return this.runtime(id).send(content)
  }

  steer(id: string, content: UserContentBlock[]): Promise<void> {
    return this.runtime(id).steer(content)
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

  private resolveDefaultModel(): ModelIntent | null {
    for (const provider of this.providers.value) {
      const model = this.models[provider.type]?.[0]
      if (model) {
        return {
          providerType: provider.type,
          modelId: model.id,
          thinkingEffort: model.reasoning?.defaultEffort ?? null,
          serviceTierId: null,
        }
      }
    }
    return null
  }

  private fallbackModel(): ModelIntent {
    return { providerType: 'claude-code', modelId: 'default', thinkingEffort: null, serviceTierId: null }
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
