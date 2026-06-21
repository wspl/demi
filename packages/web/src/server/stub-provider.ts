import type { ProviderDefinition } from '@demi/provider'
import { StubProvider, events } from '@demi/provider/testing'

const EPOCH = '1970-01-01T00:00:00.000Z'

/** A scripted provider definition for deterministic transport/dev testing without real API calls. */
export function createStubProviderDefinition(): ProviderDefinition {
  return {
    type: 'claude-code',
    displayName: 'Stub',
    state: () => ({ status: 'ready' }),
    listModels: () => ({
      providerId: 'claude-code',
      models: [
        {
          providerId: 'claude-code',
          id: 'stub-model',
          displayName: 'Stub Model',
          contextWindow: 200_000,
          outputLimit: 8_192,
          supportsTools: true,
          supportsAttachments: false,
          supportsReasoning: false,
          supportedThinkingEfforts: null,
          defaultThinkingEffort: null,
          sourceFetchedAt: EPOCH,
          stale: false,
        },
      ],
      defaultModelId: 'stub-model',
      warnings: [],
      sourceFetchedAt: EPOCH,
      stale: false,
    }),
    createProvider: () =>
      new StubProvider([
        [events.text('Hello from the stub provider.'), events.response({ inputTokens: 5, outputTokens: 6 })],
      ]),
  }
}
