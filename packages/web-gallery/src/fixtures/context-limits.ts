import { reactive } from 'vue'
import { appliedContextLimit } from '@demicodes/protocol'
import type { ContextLimitChange } from '@demicodes/web-ui/agent/context-limit'
import type { ModelInfo, ProviderInfo } from '@demicodes/web-ui/transport/protocol'

/**
 * The gallery's stand-in for the user's context limits, which the product
 * keeps in the user's preferences: one store for every specimen, so a limit
 * set in one shows in each specimen with that model, as it reaches each of
 * the user's conversations with it. Gemini starts limited to 300K.
 */
const limits = reactive(new Map<string, number>([['google/gemini-pro', 300_000]]))

function key(providerId: string, modelId: string): string {
  return `${providerId}/${modelId}`
}

/** Stores a limit the model menu chose, or removes it for the full window. */
export function setGalleryContextLimit(change: ContextLimitChange): void {
  const model = key(change.providerId, change.modelId)
  if (change.tokens === null) {
    limits.delete(model)
  } else {
    limits.set(model, change.tokens)
  }
}

/** `models` with the limits the gallery's user set, as the product's catalog adapter gives them. */
export function withContextLimits(models: Record<string, ModelInfo[]>): Record<string, ModelInfo[]> {
  return Object.fromEntries(
    Object.entries(models).map(([providerId, list]) => [
      providerId,
      list.map((model) => ({
        ...model,
        contextLimit: appliedContextLimit(model.contextWindow, limits.get(key(providerId, model.id)) ?? null),
      })),
    ]),
  )
}

export const contextLimitProviders: ProviderInfo[] = [
  { id: 'anthropic', label: 'Anthropic', isAvailable: true },
  { id: 'openai', label: 'OpenAI', isAvailable: true },
  { id: 'google', label: 'Google', isAvailable: true },
]

function model(id: string, name: string, contextWindow: number): ModelInfo {
  return {
    id,
    name,
    contextWindow,
    contextLimit: null,
    inputLimit: null,
    acceptedExtensions: ['png', 'pdf'],
    reasoning: { efforts: ['low', 'medium', 'high'], canDisable: false },
    serviceTiers: null,
  }
}

/** A model of each window size: 200K offers no limit, 800K offers 300K and 200K, 1M offers 500K too. */
export const contextLimitModels: Record<string, ModelInfo[]> = {
  anthropic: [
    model('claude-haiku', 'Claude Haiku', 200_000),
    model('claude-opus', 'Claude Opus', 1_000_000),
  ],
  openai: [model('gpt-long', 'GPT Long', 800_000)],
  google: [model('gemini-pro', 'Gemini Pro', 1_048_576)],
}
