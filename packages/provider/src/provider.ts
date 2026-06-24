import type {
  AgentProvider,
  ModelPolicy,
  Provider,
  ProviderFactoryDefinition,
  ProviderModel,
  ProviderModelList,
  ProviderRuntimeFactory,
  ProviderSelection,
} from './types'

const runtimeFactorySymbol: unique symbol = Symbol('demi.provider.runtimeFactory')

type ProviderWithRuntimeFactory = Provider & {
  [runtimeFactorySymbol]: ProviderRuntimeFactory
}

export function defineProvider(definition: ProviderFactoryDefinition): Provider {
  const { createRuntime, ...publicProvider } = definition
  return Object.freeze(
    Object.defineProperty(publicProvider, runtimeFactorySymbol, {
      value: { createRuntime },
      enumerable: false,
      configurable: false,
      writable: false,
    }),
  ) as Provider
}

export function providerRuntime(provider: Provider, selection: ProviderSelection): Promise<AgentProvider> | AgentProvider {
  const factory = (provider as Partial<ProviderWithRuntimeFactory>)[runtimeFactorySymbol]
  if (!factory) throw new Error(`Provider "${provider.id}" does not expose a runtime factory`)
  return factory.createRuntime(selection)
}

export function applyModelPolicy(
  list: ProviderModelList,
  providerId: string,
  policy: ModelPolicy | undefined,
): ProviderModelList {
  const include = policy?.include ? new Set(policy.include) : null
  const exclude = policy?.exclude ? new Set(policy.exclude) : null
  const models = list.models
    .filter((model) => (!include || include.has(model.id)) && (!exclude || !exclude.has(model.id)))
    .map((model) => withProviderId(model, providerId))

  const defaultModelId =
    policy?.default && models.some((model) => model.id === policy.default)
      ? policy.default
      : list.defaultModelId && models.some((model) => model.id === list.defaultModelId)
        ? list.defaultModelId
        : null

  return {
    ...list,
    providerId,
    models,
    defaultModelId,
  }
}

function withProviderId(model: ProviderModel, providerId: string): ProviderModel {
  return model.providerId === providerId ? model : { ...model, providerId }
}
