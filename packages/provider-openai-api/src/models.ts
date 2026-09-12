import { modelListFromConfiguredModels, STATIC_CATALOG_SOURCE_DATE, type ConfiguredModelOptions, type ProviderModelList } from '@demicodes/provider'

export type OpenAIApiModelOptions = ConfiguredModelOptions

export function openAIApiDefaultModels(providerId = 'openai'): ProviderModelList {
  return modelListFromConfiguredModels(
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
      sourceFetchedAt: STATIC_CATALOG_SOURCE_DATE,
    },
  )
}
