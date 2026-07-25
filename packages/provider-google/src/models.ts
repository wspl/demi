import type { ProviderModel, ProviderModelList } from '@demicodes/provider'

export interface GoogleModelOptions {
  id: string
  displayName?: string
  description?: string
  contextWindow: number
  outputLimit?: number | null
  supportsTools?: boolean | null
  supportsAttachments?: boolean | null
  /** Gemini takes video natively (inline `inlineData` parts), audio track included. */
  supportsVideo?: boolean | null
  supportsReasoning?: boolean | null
  supportedThinkingEfforts?: string[] | null
  defaultThinkingEffort?: string | null
  canDisableThinking?: boolean | null
  serviceTiers?: ProviderModel['serviceTiers']
  defaultServiceTierId?: string | null
}

const SOURCE_FETCHED_AT = '1970-01-01T00:00:00.000Z'
const EFFORTS = ['low', 'medium', 'high', 'xhigh', 'max']

export function googleDefaultModels(providerId = 'google'): ProviderModelList {
  return modelListFromGoogleModels(
    [
      {
        id: 'gemini-3.6-flash',
        displayName: 'Gemini 3.6 Flash',
        contextWindow: 1_048_576,
        outputLimit: 65_536,
        supportsTools: true,
        supportsAttachments: true,
        supportsVideo: true,
        supportsReasoning: true,
        supportedThinkingEfforts: EFFORTS,
        defaultThinkingEffort: 'medium',
      },
      {
        id: 'gemini-3.5-flash',
        displayName: 'Gemini 3.5 Flash',
        contextWindow: 1_048_576,
        outputLimit: 65_536,
        supportsTools: true,
        supportsAttachments: true,
        supportsVideo: true,
        supportsReasoning: true,
        supportedThinkingEfforts: EFFORTS,
        defaultThinkingEffort: 'medium',
      },
      {
        id: 'gemini-3.1-pro-preview',
        displayName: 'Gemini 3.1 Pro Preview',
        contextWindow: 1_048_576,
        outputLimit: 65_536,
        supportsTools: true,
        supportsAttachments: true,
        supportsVideo: true,
        supportsReasoning: true,
        supportedThinkingEfforts: EFFORTS,
        defaultThinkingEffort: 'medium',
      },
      {
        id: 'gemini-2.5-flash',
        displayName: 'Gemini 2.5 Flash',
        contextWindow: 1_048_576,
        outputLimit: 65_536,
        supportsTools: true,
        supportsAttachments: true,
        supportsVideo: true,
        supportsReasoning: true,
        supportedThinkingEfforts: EFFORTS,
        defaultThinkingEffort: 'medium',
      },
    ],
    {
      providerId,
      defaultModelId: 'gemini-3.6-flash',
      sourceFetchedAt: SOURCE_FETCHED_AT,
    },
  )
}

export function modelListFromGoogleModels(
  models: GoogleModelOptions[],
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
    supportsVideo: model.supportsVideo ?? null,
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
