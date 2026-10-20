import { contextLimits } from '@demicodes/protocol'
import type { ModelInfo } from '../transport/protocol'
import { formatTokens } from '../ui/token-count'
import { t } from './locales/text'

/**
 * One model's context limit as the model menu changes it: the user's, for every
 * conversation with that model. Null tokens remove the limit, so the model uses
 * its full window.
 */
export interface ContextLimitChange {
  providerId: string
  modelId: string
  tokens: number | null
}

/**
 * One option of the Context row: the window it gives, and the limit it stores,
 * null for the full window.
 */
export interface ContextLimitOption {
  window: number
  tokens: number | null
}

/**
 * The window Demi uses for `model`: the user's limit on it, or its context
 * window; null when unknown.
 */
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

/**
 * One option of a subagent profile's Context: its label, and the tokens the
 * profile stores, null for the user's limit.
 */
export interface ProfileContextOption {
  label: string
  tokens: number | null
}

/**
 * A subagent profile's Context options on `model`
 * (`crates/agent-server/src/server/tree/profiles.rs`):
 *
 * - Default, the user's limit on the model, with the window it gives now
 * - the Context row's options, the full window stored as its own size
 *
 * None for a model without a Context row.
 */
export function profileContextOptions(model: ModelInfo): ProfileContextOption[] {
  const options = contextLimitOptions(model)
  if (options.length === 0) {
    return []
  }
  return [
    { label: t('models.default', { window: formatTokens(contextWindowInUse(model)) }), tokens: null },
    ...options.map((option) => ({ label: formatTokens(option.window), tokens: option.window })),
  ]
}

/**
 * The profile's Context `tokens` as `model` offers it, null for Default: a
 * choice the window no longer offers is the full window, as for a session.
 */
export function profileContextChoice(model: ModelInfo, tokens: number | null): number | null {
  const offered = profileContextOptions(model).some((option) => option.tokens === tokens)
  return tokens === null || offered ? tokens : model.contextWindow
}
