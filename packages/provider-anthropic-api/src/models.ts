import type { ProviderModel, ProviderModelList } from '@demi/provider'

export interface AnthropicApiModelOptions {
  id: string
  displayName?: string
  description?: string
  contextWindow: number
  outputLimit?: number | null
  supportsTools?: boolean | null
  supportsAttachments?: boolean | null
  supportsReasoning?: boolean | null
  supportedThinkingEfforts?: string[] | null
  defaultThinkingEffort?: string | null
  canDisableThinking?: boolean | null
  serviceTiers?: ProviderModel['serviceTiers']
  defaultServiceTierId?: string | null
}

const SOURCE_FETCHED_AT = '1970-01-01T00:00:00.000Z'

export function anthropicApiDefaultModels(providerId = 'anthropic'): ProviderModelList {
  return modelListFromAnthropicApiModels(
    [
      {
        id: 'claude-opus-4-8',
        displayName: 'Claude Opus 4.8',
        contextWindow: 1_000_000,
        outputLimit: 128_000,
        supportsTools: true,
        supportsAttachments: true,
        supportsReasoning: true,
        supportedThinkingEfforts: ['low', 'medium', 'high', 'xhigh', 'max'],
        canDisableThinking: false,
      },
      {
        id: 'claude-opus-4-7',
        displayName: 'Claude Opus 4.7',
        contextWindow: 1_000_000,
        outputLimit: 128_000,
        supportsTools: true,
        supportsAttachments: true,
        supportsReasoning: true,
        supportedThinkingEfforts: ['low', 'medium', 'high', 'xhigh', 'max'],
        canDisableThinking: false,
      },
      {
        id: 'claude-opus-4-6',
        displayName: 'Claude Opus 4.6',
        contextWindow: 1_000_000,
        outputLimit: 128_000,
        supportsTools: true,
        supportsAttachments: true,
        supportsReasoning: true,
        supportedThinkingEfforts: ['low', 'medium', 'high', 'max'],
        canDisableThinking: false,
      },
      {
        id: 'claude-sonnet-4-6',
        displayName: 'Claude Sonnet 4.6',
        contextWindow: 1_000_000,
        outputLimit: 64_000,
        supportsTools: true,
        supportsAttachments: true,
        supportsReasoning: true,
        supportedThinkingEfforts: ['low', 'medium', 'high', 'max'],
        canDisableThinking: false,
      },
      {
        id: 'claude-fable-5',
        displayName: 'Claude Fable 5',
        contextWindow: 1_000_000,
        outputLimit: 128_000,
        supportsTools: true,
        supportsAttachments: true,
        supportsReasoning: true,
        supportedThinkingEfforts: ['low', 'medium', 'high', 'xhigh', 'max'],
        canDisableThinking: false,
      },
    ],
    {
      providerId,
      defaultModelId: 'claude-opus-4-8',
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
    contextWindow: positiveInteger(model.contextWindow, `models[${model.id}].contextWindow`),
    outputLimit: model.outputLimit ?? null,
    supportsTools: model.supportsTools ?? null,
    supportsAttachments: model.supportsAttachments ?? null,
    supportsReasoning: model.supportsReasoning ?? null,
    supportedThinkingEfforts: model.supportedThinkingEfforts ? [...model.supportedThinkingEfforts] : null,
    defaultThinkingEffort: model.defaultThinkingEffort ?? null,
    canDisableThinking: model.canDisableThinking ?? null,
    serviceTiers: model.serviceTiers ? model.serviceTiers.map((tier) => ({ ...tier })) : model.serviceTiers,
    defaultServiceTierId: model.defaultServiceTierId ?? null,
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

function positiveInteger(value: number, field: string): number {
  if (!Number.isInteger(value) || value <= 0) throw new Error(`${field} must be a positive integer`)
  return value
}
