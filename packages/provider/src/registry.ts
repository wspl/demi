import type { AgentProvider, ProviderDefinition, ProviderRuntimeState } from './types'

export interface ProviderRegistrySnapshot {
  providers: ProviderDefinition[]
}

export type ProviderRegistryListener = (snapshot: ProviderRegistrySnapshot) => void

export class ProviderRegistry {
  private readonly providers = new Map<string, ProviderDefinition>()
  private readonly listeners = new Set<ProviderRegistryListener>()

  register(definition: ProviderDefinition): () => void {
    if (this.providers.has(definition.type)) {
      throw new Error(`ProviderRegistry: provider "${definition.type}" is already registered`)
    }
    this.providers.set(definition.type, definition)
    this.emit()
    return () => this.unregister(definition.type)
  }

  unregister(type: string): boolean {
    const removed = this.providers.delete(type)
    if (removed) this.emit()
    return removed
  }

  get(type: string): ProviderDefinition | null {
    return this.providers.get(type) ?? null
  }

  list(): ProviderDefinition[] {
    return [...this.providers.values()]
  }

  async createProvider(type: string, config: unknown): Promise<AgentProvider> {
    const definition = this.providers.get(type)
    if (!definition) {
      throw new Error(`ProviderRegistry: provider "${type}" is not registered`)
    }
    return definition.createProvider(config)
  }

  async state(type: string): Promise<ProviderRuntimeState> {
    const definition = this.providers.get(type)
    if (!definition) {
      return { status: 'unavailable', message: `Provider "${type}" is not registered` }
    }
    return definition.state?.() ?? { status: 'unknown' }
  }

  observe(listener: ProviderRegistryListener): () => void {
    this.listeners.add(listener)
    listener(this.snapshot())
    return () => {
      this.listeners.delete(listener)
    }
  }

  snapshot(): ProviderRegistrySnapshot {
    return { providers: this.list() }
  }

  private emit(): void {
    const snapshot = this.snapshot()
    for (const listener of this.listeners) listener(snapshot)
  }
}
