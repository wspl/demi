import type { ContextUsage } from '@demicodes/protocol'
import type { ModelInfo, ProviderInfo } from '@demicodes/web-ui/transport/protocol'

export const demoProviders: ProviderInfo[] = [
  { id: 'anthropic', label: 'Anthropic', isAvailable: true },
  { id: 'openai', label: 'OpenAI', isAvailable: true },
]

export const offlineProviders: ProviderInfo[] = demoProviders.map((provider) => ({
  ...provider,
  isAvailable: false,
}))

export const openaiOnlyProviders: ProviderInfo[] = [
  { id: 'anthropic', label: 'Anthropic', isAvailable: false },
  { id: 'openai', label: 'OpenAI', isAvailable: true },
]

const fastTier = [{ id: 'priority', label: 'Fast', fast: true }]

function model(partial: Pick<ModelInfo, 'id' | 'name'> & Partial<ModelInfo>): ModelInfo {
  return {
    contextWindow: 200_000,
    contextLimit: null,
    acceptedExtensions: ['png', 'pdf', 'md'],
    reasoning: null,
    serviceTiers: null,
    ...partial,
  }
}

export const demoModels: Record<string, ModelInfo[]> = {
  anthropic: [
    model({
      id: 'claude-sonnet',
      name: 'Claude Sonnet',
      reasoning: {
        efforts: ['low', 'medium', 'high', 'max'],
        canDisable: true,
      },
      serviceTiers: fastTier,
    }),
    model({
      id: 'claude-opus',
      name: 'Claude Opus',
      // Over 500K: its menu offers the Context row.
      contextWindow: 1_000_000,
      reasoning: {
        efforts: ['low', 'medium', 'high'],
        canDisable: false,
      },
    }),
  ],
  openai: [
    model({ id: 'gpt-5', name: 'GPT-5', serviceTiers: fastTier }),
  ],
}

/** The window the gallery's conversations use, as the backend reports it. */
const DEMO_WINDOW = 200_000

/** The usage the backend reports at `tokens` of `window`, the window in use: Compact is taken from half. */
export function usageOf(tokens: number, window: number): ContextUsage {
  return {
    tokens,
    window,
    compactFrom: Math.floor(window / 2),
  }
}

/** The backend's usage at `ratio` of the gallery conversations' window. */
export function usageAt(ratio: number): ContextUsage {
  return usageOf(Math.round(DEMO_WINDOW * ratio), DEMO_WINDOW)
}
