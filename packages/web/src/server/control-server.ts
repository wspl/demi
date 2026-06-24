import type { Provider, ProviderModel, ProviderSelection } from '@demi/provider'
import type {
  ControlMethod,
  ModelInfo,
  PrepareSessionParams,
  ProviderInfo,
  WorkspaceInfo,
} from '@demi/web-ui/transport/protocol'
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
    const explicitModel = this.explicitModelFor(params.providerId)
    if (explicitModel) {
      const model = await this.findProviderCatalogModel(provider, explicitModel.id)
      return [model ? withModelInfoDisplayName(toModelInfo(model), explicitModel.displayName) : explicitModelInfo(explicitModel)]
    }
    if (!provider.listModels) throw new Error(`Provider "${params.providerId}" does not expose a model catalog`)
    const catalog = await provider.listModels()
    return catalog.models.map(toModelInfo)
  }

  private async prepareSession(params: PrepareSessionParams): Promise<ProviderSelection> {
    const catalogModel = await this.findCatalogModel(params.providerId, params.modelId)
    const model = buildModelSelection(
      params.providerId,
      params.modelId,
      params.thinkingEffort ?? null,
      params.serviceTierId ?? null,
      catalogModel,
    )
    return { providerId: params.providerId, model }
  }

  private async findCatalogModel(providerId: string, modelId: string) {
    try {
      const provider = this.providerFor(providerId)
      const explicitModel = this.explicitModelFor(providerId)
      if (explicitModel && modelId !== explicitModel.id) return null
      const model = await this.findProviderCatalogModel(provider, modelId)
      if (!explicitModel) return model
      return model
        ? withProviderModelDisplayName(model, explicitModel.displayName)
        : explicitProviderModel(providerId, explicitModel)
    } catch {
      const explicitModel = this.explicitModelFor(providerId)
      return explicitModel?.id === modelId ? explicitProviderModel(providerId, explicitModel) : null
    }
  }

  private providerFor(providerId: string): Provider {
    const provider = this.providers.get(providerId)
    if (!provider) throw new Error(`Provider "${providerId}" is not available`)
    return provider
  }

  private explicitModelFor(providerId: string): ExplicitModelOverride | null {
    if (this.options.provider !== providerId || !this.options.modelId) return null
    return { id: this.options.modelId, displayName: this.options.modelDisplayName }
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

interface ExplicitModelOverride {
  id: string
  displayName: string | null
}

function explicitModelInfo(model: ExplicitModelOverride): ModelInfo {
  return {
    id: model.id,
    name: model.displayName ?? model.id,
    contextWindow: null,
    inputLimit: null,
    acceptedExtensions: [],
    reasoning: null,
  }
}

function explicitProviderModel(providerId: string, model: ExplicitModelOverride): ProviderModel {
  return {
    providerId,
    id: model.id,
    displayName: model.displayName ?? model.id,
    contextWindow: null,
    outputLimit: null,
    supportsTools: null,
    supportsAttachments: null,
    supportsReasoning: null,
    supportedThinkingEfforts: null,
    defaultThinkingEffort: null,
    sourceFetchedAt: '1970-01-01T00:00:00.000Z',
    stale: false,
  }
}

function withModelInfoDisplayName(model: ModelInfo, displayName: string | null): ModelInfo {
  return displayName ? { ...model, name: displayName } : model
}

function withProviderModelDisplayName(model: ProviderModel, displayName: string | null): ProviderModel {
  return displayName ? { ...model, displayName } : model
}
