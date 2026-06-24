import { defineProvider, type Provider } from '@demi/provider'
import { StubProvider, events } from '@demi/provider/testing'

const EPOCH = '1970-01-01T00:00:00.000Z'

/** A scripted provider for deterministic transport/dev testing without real API calls. */
export function createStubProvider(): Provider {
  return defineProvider({
    id: 'claude-code',
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
    createRuntime: () =>
      new StubProvider([
        [
          { type: 'thinking_delta', text: 'Let me inspect the workspace before answering.' },
          { type: 'thinking_signature', signature: 'stub-signature' },
          events.toolCall('tool-1', 'shell_exec', { script: 'echo "hello from demi" && ls -a' }),
        ],
        [
          events.text('Hello from the stub provider. The shell command ran and its output is shown above.'),
          events.response({ inputTokens: 1280, outputTokens: 48, cacheReadTokens: 920 }),
        ],
      ]),
  })
}
