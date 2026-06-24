import type { ProviderModel, ProviderModelList } from '@demi/provider'

export interface AnthropicApiModelOptions {
  id: string
  displayName?: string
  description?: string
  contextWindow?: number | null
  outputLimit?: number | null
  supportsTools?: boolean | null
  supportsAttachments?: boolean | null
  supportsReasoning?: boolean | null
  supportedThinkingEfforts?: string[] | null
  defaultThinkingEffort?: string | null
  canDisableThinking?: boolean | null
}

const SOURCE_FETCHED_AT = '1970-01-01T00:00:00.000Z'

export function anthropicApiDefaultModels(providerId = 'anthropic'): ProviderModelList {
  return modelListFromAnthropicApiModels(
    [
      {
        id: 'claude-opus-4-8',
        displayName: 'Claude Opus 4.8',
        contextWindow: 200_000,
        outputLimit: 32_000,
        supportsTools: true,
        supportsAttachments: true,
        supportsReasoning: true,
        supportedThinkingEfforts: null,
      },
      {
        id: 'claude-sonnet-4-8',
        displayName: 'Claude Sonnet 4.8',
        contextWindow: 200_000,
        outputLimit: 64_000,
        supportsTools: true,
        supportsAttachments: true,
        supportsReasoning: true,
        supportedThinkingEfforts: null,
      },
      {
        id: 'claude-haiku-4-5',
        displayName: 'Claude Haiku 4.5',
        contextWindow: 200_000,
        outputLimit: 32_000,
        supportsTools: true,
        supportsAttachments: true,
        supportsReasoning: true,
        supportedThinkingEfforts: null,
      },
    ],
    {
      providerId,
      defaultModelId: 'claude-sonnet-4-8',
      sourceFetchedAt: SOURCE_FETCHED_AT,
    },
  )
}

export function modelListFromAnthropicApiModels(
  models: AnthropicApiModelOptions[],
  options: {
    providerId: string
    defaultModelId?: string | null
    sourceFetchedAt?: string
    stale?: boolean
  },
): ProviderModelList {
  const sourceFetchedAt = options.sourceFetchedAt ?? new Date().toISOString()
  const stale = options.stale === true
  const mapped = models.map((model): ProviderModel => ({
    providerId: options.providerId,
    id: model.id,
    displayName: model.displayName ?? model.id,
    description: model.description,
    contextWindow: model.contextWindow ?? null,
    outputLimit: model.outputLimit ?? null,
    supportsTools: model.supportsTools ?? null,
    supportsAttachments: model.supportsAttachments ?? null,
    supportsReasoning: model.supportsReasoning ?? null,
    supportedThinkingEfforts: model.supportedThinkingEfforts ? [...model.supportedThinkingEfforts] : null,
    defaultThinkingEffort: model.defaultThinkingEffort ?? null,
    canDisableThinking: model.canDisableThinking ?? null,
    sourceFetchedAt,
    stale,
  }))
  return {
    providerId: options.providerId,
    models: mapped,
    defaultModelId:
      options.defaultModelId && mapped.some((model) => model.id === options.defaultModelId)
        ? options.defaultModelId
        : mapped[0]?.id ?? null,
    warnings: [],
    sourceFetchedAt,
    stale,
  }
}
