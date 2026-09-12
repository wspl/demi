import { modelListFromConfiguredModels, STATIC_CATALOG_SOURCE_DATE, type ConfiguredModelOptions, type ProviderModelList } from '@demicodes/provider'

export type AnthropicApiModelOptions = ConfiguredModelOptions

export function anthropicApiDefaultModels(
  providerId = 'anthropic'
): ProviderModelList {
  return modelListFromConfiguredModels(
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
      sourceFetchedAt: STATIC_CATALOG_SOURCE_DATE,
    },
  )
}
