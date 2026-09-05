import type { ThinkingConfig, TokenUsage } from '@demicodes/core'
import type { ModelInfo, ProviderInfo } from '@demicodes/web-ui/transport/protocol'

export const demoProviders: ProviderInfo[] = [
  { id: 'anthropic', label: 'Anthropic', isAvailable: true },
  { id: 'openai', label: 'OpenAI', isAvailable: true },
]

const fastTier = [{ id: 'priority', label: 'Fast', fast: true }]

function model(partial: Pick<ModelInfo, 'id' | 'name'> & Partial<ModelInfo>): ModelInfo {
  return {
    contextWindow: 200_000,
    inputLimit: 180_000,
    acceptedExtensions: ['.png', '.pdf', '.md'],
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
        defaultEffort: 'medium',
        canDisable: true,
      },
      serviceTiers: fastTier,
    }),
    model({
      id: 'claude-opus',
      name: 'Claude Opus',
      reasoning: {
        efforts: ['low', 'medium', 'high'],
        defaultEffort: 'high',
        canDisable: false,
      },
    }),
  ],
  openai: [
    model({ id: 'gpt-5', name: 'GPT-5', serviceTiers: fastTier }),
  ],
}

export const mediumThinking: ThinkingConfig = { type: 'effort', effort: 'medium', summary: null }

export function usageAt(ratio: number): TokenUsage {
  const used = Math.round(180_000 * ratio)
  return {
    inputTokens: used,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheWriteTokens: 0,
  }
}
