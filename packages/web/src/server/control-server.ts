import type { ProviderConfig } from '@demi/agent'
import type { ProviderRegistry } from '@demi/provider'
import type {
  ControlMethod,
  ModelInfo,
  PrepareSessionParams,
  ProviderInfo,
  WorkspaceInfo,
} from '@demi/web-ui/transport/protocol'
import { providerConfigFor, type ServerOptions } from './server-options'
import { buildModelSelection, toModelInfo } from './web-model'

export class ControlServer {
  constructor(
    private readonly registry: ProviderRegistry,
    private readonly options: ServerOptions,
  ) {}

  handle(method: ControlMethod, params: unknown): Promise<unknown> {
    switch (method) {
      case 'listProviders':
        return this.listProviders()
      case 'listModels':
        return this.listModels(params as { providerType: string })
      case 'prepareSession':
        return this.prepareSession(params as PrepareSessionParams)
      case 'defaultWorkspace':
        return Promise.resolve(this.defaultWorkspace())
    }
  }

  private async listProviders(): Promise<ProviderInfo[]> {
    const providers: ProviderInfo[] = []
    for (const definition of this.registry.list()) {
      const state = await this.registry.state(definition.type)
      providers.push({
        type: definition.type,
        label: definition.displayName,
        isAvailable: state.status === 'ready' || state.status === 'unknown',
      })
    }
    return providers
  }

  private async listModels(params: { providerType: string }): Promise<ModelInfo[]> {
    const config = providerConfigFor(params.providerType, this.options)
    const catalog = await this.registry.listModels(params.providerType, config)
    return catalog.models.map(toModelInfo)
  }

  private async prepareSession(params: PrepareSessionParams): Promise<ProviderConfig> {
    const config = providerConfigFor(params.providerType, this.options)
    const catalogModel = await this.findCatalogModel(params.providerType, params.modelId, config)
    const model = buildModelSelection(
      params.providerType,
      params.modelId,
      params.thinkingEffort ?? null,
      params.serviceTierId ?? null,
      catalogModel,
    )
    return { type: params.providerType, config, model }
  }

  private async findCatalogModel(providerType: string, modelId: string, config: Record<string, unknown>) {
    try {
      const catalog = await this.registry.listModels(providerType, config)
      return catalog.models.find((model) => model.id === modelId) ?? null
    } catch {
      return null
    }
  }

  private defaultWorkspace(): WorkspaceInfo {
    return { cwd: this.options.cwd }
  }
}
