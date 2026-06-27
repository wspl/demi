import type { Provider, ProviderModel, ProviderSelection } from '@demicodes/provider'
import type {
  ControlMethod,
  ModelInfo,
  PrepareSessionParams,
  ProviderInfo,
  WorkspaceInfo,
} from '@demicodes/web-ui/transport/protocol'
import type { ServerOptions } from './server-options'
import { buildModelSelection, toModelInfo } from './web-model'

export class ControlServer {
  private readonly providers: Map<string, Provider>

  constructor(
    providers: readonly Provider[],
    private readonly options: ServerOptions,
  ) {
    this.providers = new Map(providers.map((provider) => [provider.id, provider]))
  }

  handle(method: ControlMethod, params: unknown): Promise<unknown> {
    switch (method) {
      case 'listProviders':
        return this.listProviders()
      case 'listModels':
        return this.listModels(params as { providerId: string })
      case 'prepareSession':
        return this.prepareSession(params as PrepareSessionParams)
      case 'defaultWorkspace':
        return Promise.resolve(this.defaultWorkspace())
    }
  }

  private async listProviders(): Promise<ProviderInfo[]> {
    const providers: ProviderInfo[] = []
    for (const provider of this.providers.values()) {
      const state = await provider.state?.() ?? { status: 'unknown' }
      providers.push({
        id: provider.id,
        label: provider.displayName,
        isAvailable: state.status === 'ready' || state.status === 'unknown',
      })
    }
    return providers
  }

  private async listModels(params: { providerId: string }): Promise<ModelInfo[]> {
    const provider = this.providerFor(params.providerId)
    if (!provider.listModels) throw new Error(`Provider "${params.providerId}" does not expose a model catalog`)
    const catalog = await provider.listModels()
    return catalog.models.map(toModelInfo)
  }

  private async prepareSession(params: PrepareSessionParams): Promise<ProviderSelection> {
    const catalogModel = await this.findCatalogModel(params.providerId, params.modelId)
    const thinkingEffort = hasOwn(params, 'thinkingEffort')
      ? params.thinkingEffort ?? null
      : defaultThinkingEffort(catalogModel)
    const model = buildModelSelection(
      params.providerId,
      params.modelId,
      thinkingEffort,
      params.serviceTierId ?? null,
      catalogModel,
    )
    return { providerId: params.providerId, model }
  }

  private async findCatalogModel(providerId: string, modelId: string) {
    try {
      const provider = this.providerFor(providerId)
      return await this.findProviderCatalogModel(provider, modelId)
    } catch {
      return null
    }
  }

  private providerFor(providerId: string): Provider {
    const provider = this.providers.get(providerId)
    if (!provider) throw new Error(`Provider "${providerId}" is not available`)
    return provider
  }

  private async findProviderCatalogModel(provider: Provider, modelId: string): Promise<ProviderModel | null> {
    try {
      if (!provider.listModels) return null
      const catalog = await provider.listModels()
      return catalog.models.find((model) => model.id === modelId) ?? null
    } catch {
      return null
    }
  }

  private defaultWorkspace(): WorkspaceInfo {
    return { cwd: this.options.cwd }
  }
}

function defaultThinkingEffort(model: ProviderModel | null): string | null {
  return model?.defaultThinkingEffort ?? (model?.canDisableThinking === false ? model.supportedThinkingEfforts?.[0] ?? null : null)
}

function hasOwn<T extends object, K extends PropertyKey>(object: T, key: K): object is T & Record<K, unknown> {
  return Object.prototype.hasOwnProperty.call(object, key)
}
