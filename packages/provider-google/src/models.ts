import { modelListFromConfiguredModels, STATIC_CATALOG_SOURCE_DATE, type ConfiguredModelOptions, type ProviderModelList } from '@demicodes/provider'

export type GoogleModelOptions = ConfiguredModelOptions

const EFFORTS = ['low', 'medium', 'high', 'xhigh', 'max']

export function googleDefaultModels(providerId = 'google'): ProviderModelList {
  return modelListFromConfiguredModels(
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
      sourceFetchedAt: STATIC_CATALOG_SOURCE_DATE,
    },
  )
}
