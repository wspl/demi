import { contextLimits } from '@demicodes/protocol'
import type { ModelInfo } from '../transport/protocol'

/**
 * One model's context limit as the model menu changes it (`models.md`
 * § Context limit): the user's, for every conversation with that model. Null
 * tokens remove the limit, so the model uses its full window.
 */
export interface ContextLimitChange {
  providerId: string
  modelId: string
  tokens: number | null
}

/** One option of the Context row: the window it gives, and the limit it stores, null for the full window. */
export interface ContextLimitOption {
  window: number
  tokens: number | null
}

/** The window Demi uses for `model`: the user's limit on it, or its context window; null when unknown. */
export function contextWindowInUse(model: ModelInfo | null | undefined): number | null {
  return model?.contextLimit ?? model?.contextWindow ?? null
}

/**
 * The Context row's options for `model`: its full window first, then the
 * limits its window offers. None for a window of at most 500K, which shows
 * no row.
 */
export function contextLimitOptions(model: ModelInfo): ContextLimitOption[] {
  const full = model.contextWindow
  const limits = contextLimits(full)
  if (full === null || limits.length === 0) {
    return []
  }
  return [
    { window: full, tokens: null },
    ...limits.map((tokens) => ({ window: tokens, tokens })),
  ]
}
