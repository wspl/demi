import { z } from 'zod'
import { modelLimitsSchema } from '@demicodes/provider'
import type { ProviderModel, ProviderModelList } from '@demicodes/provider'

const serviceTierSchema = z.object({
  id: z.string().min(1),
  label: z.string().min(1),
  description: z.string().optional(),
  fast: z.boolean(),
})

/**
 * One model of an OpenAI-compatible endpoint, as its configuration states it.
 * The token limits come from the shared `modelLimitsSchema`.
 */
export const openAIApiModelOptionsSchema = modelLimitsSchema.extend({
  id: z.string().min(1),
  displayName: z.string().min(1).optional(),
  description: z.string().optional(),
  supportsTools: z.boolean().nullish(),
  supportsAttachments: z.boolean().nullish(),
  supportsReasoning: z.boolean().nullish(),
  supportedThinkingEfforts: z.array(z.string().min(1)).nullish(),
  defaultThinkingEffort: z.string().min(1).nullish(),
  canDisableThinking: z.boolean().nullish(),
  serviceTiers: z.array(serviceTierSchema).nullish(),
  defaultServiceTierId: z.string().min(1).nullish(),
})

export type OpenAIApiModelOptions = z.infer<typeof openAIApiModelOptionsSchema>

const SOURCE_FETCHED_AT = '1970-01-01T00:00:00.000Z'

export function openAIApiDefaultModels(providerId = 'openai'): ProviderModelList {
  return modelListFromOpenAIApiModels(
    [
      {
        id: 'gpt-5.5',
        displayName: 'GPT-5.5',
        contextWindow: 272_000,
        outputLimit: null,
        supportsTools: false,
        supportsAttachments: true,
        supportsReasoning: true,
        supportedThinkingEfforts: ['low', 'medium', 'high', 'xhigh'],
        serviceTiers: [{
          id: 'priority',
          label: 'Fast',
          description: '1.5x speed, increased usage',
          fast: true
        }],
      },
      {
        id: 'gpt-5.4',
        displayName: 'GPT-5.4',
        contextWindow: 272_000,
        outputLimit: null,
        supportsTools: false,
        supportsAttachments: true,
        supportsReasoning: true,
        supportedThinkingEfforts: ['low', 'medium', 'high', 'xhigh'],
        serviceTiers: [{
          id: 'priority',
          label: 'Fast',
          description: '1.5x speed, increased usage',
          fast: true
        }],
      },
      {
        id: 'gpt-5.4-mini',
        displayName: 'GPT-5.4-Mini',
        contextWindow: 272_000,
        outputLimit: null,
        supportsTools: false,
        supportsAttachments: true,
        supportsReasoning: true,
        supportedThinkingEfforts: ['low', 'medium', 'high', 'xhigh'],
        serviceTiers: [],
      },
      {
        id: 'gpt-5.3-codex-spark',
        displayName: 'GPT-5.3-Codex-Spark',
        contextWindow: 128_000,
        outputLimit: null,
        supportsTools: false,
        supportsAttachments: false,
        supportsReasoning: true,
        supportedThinkingEfforts: ['low', 'medium', 'high', 'xhigh'],
        serviceTiers: [],
      },
    ],
    {
      providerId,
      defaultModelId: 'gpt-5.5',
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
  const mapped = models.map((entry): ProviderModel => {
    const model = openAIApiModelOptionsSchema.parse(entry)
    return {
      providerId: options.providerId,
      id: model.id,
      displayName: model.displayName ?? model.id,
      description: model.description,
      contextWindow: model.contextWindow,
      outputLimit: model.outputLimit ?? null,
      supportsTools: model.supportsTools ?? null,
      supportsAttachments: model.supportsAttachments ?? null,
      supportsReasoning: model.supportsReasoning ?? null,
      supportedThinkingEfforts: model.supportedThinkingEfforts
        ? [...model.supportedThinkingEfforts]
        : null,
      defaultThinkingEffort: model.defaultThinkingEffort ?? null,
      canDisableThinking: model.canDisableThinking ?? null,
      serviceTiers: model.serviceTiers
        ? model.serviceTiers.map((tier) => ({ ...tier }))
        : model.serviceTiers,
      defaultServiceTierId: model.defaultServiceTierId ?? null,
      sourceFetchedAt,
      stale,
    }
  })
  return {
    providerId: options.providerId,
    models: mapped,
    defaultModelId:
      options.defaultModelId
        && mapped.some((model) => model.id === options.defaultModelId)
        ? options.defaultModelId
        : mapped[0]?.id ?? null,
    warnings: [],
    sourceFetchedAt,
    stale,
  }
}
