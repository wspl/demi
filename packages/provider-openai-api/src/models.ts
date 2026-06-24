import type { ProviderModel, ProviderModelList } from '@demi/provider'

export interface OpenAIApiModelOptions {
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

export function openAIApiDefaultModels(providerId = 'openai'): ProviderModelList {
  return modelListFromOpenAIApiModels(
    [
      {
        id: 'gpt-5.1',
        displayName: 'GPT-5.1',
        contextWindow: 400_000,
        outputLimit: 128_000,
        supportsTools: true,
        supportsAttachments: true,
        supportsReasoning: true,
        supportedThinkingEfforts: ['minimal', 'low', 'medium', 'high'],
        defaultThinkingEffort: 'medium',
      },
      {
        id: 'gpt-5',
        displayName: 'GPT-5',
        contextWindow: 400_000,
        outputLimit: 128_000,
        supportsTools: true,
        supportsAttachments: true,
        supportsReasoning: true,
        supportedThinkingEfforts: ['minimal', 'low', 'medium', 'high'],
        defaultThinkingEffort: 'medium',
      },
      {
        id: 'gpt-4.1',
        displayName: 'GPT-4.1',
        contextWindow: 1_000_000,
        outputLimit: 32_768,
        supportsTools: true,
        supportsAttachments: true,
        supportsReasoning: false,
        supportedThinkingEfforts: null,
      },
      {
        id: 'gpt-4.1-mini',
        displayName: 'GPT-4.1 mini',
        contextWindow: 1_000_000,
        outputLimit: 32_768,
        supportsTools: true,
        supportsAttachments: true,
        supportsReasoning: false,
        supportedThinkingEfforts: null,
      },
    ],
    {
      providerId,
      defaultModelId: 'gpt-5.1',
      sourceFetchedAt: SOURCE_FETCHED_AT,
    },
  )
}

export function modelListFromOpenAIApiModels(
  models: OpenAIApiModelOptions[],
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
