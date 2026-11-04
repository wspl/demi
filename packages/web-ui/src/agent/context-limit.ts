import { contextLimits } from '@demicodes/protocol'
import type { ModelInfo } from '../transport/protocol'
import { formatDecimal } from '../ui/numbers'
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
 * limits the menu offers on its window, with `kept`, a size stored outside
 * them, in its place by size, so the row shows what applies. None for a
 * window under 500K that keeps no such size, which shows no row.
 */
export function contextLimitOptions(model: ModelInfo, kept: number | null): ContextLimitOption[] {
  const full = model.contextWindow
  if (full === null) {
    return []
  }
  const limits = contextLimits(full)
  const sizes = kept !== null && kept < full && !limits.includes(kept)
    ? [...limits, kept].sort((larger, smaller) => smaller - larger)
    : limits
  if (sizes.length === 0) {
    return []
  }
  return [
    { window: full, tokens: null },
    ...sizes.map((tokens) => ({ window: tokens, tokens })),
  ]
}

/**
 * Each option's label: its window as the UI rounds token counts, or the whole
 * count where rounding would read as another option's, as a limit of 1,000,000
 * beside a window of 1,048,576 would.
 */
export function contextOptionLabels(options: readonly ContextLimitOption[]): string[] {
  const rounded = options.map((option) => formatTokens(option.window))
  return options.map((option, index) =>
    rounded.indexOf(rounded[index]) === rounded.lastIndexOf(rounded[index])
      ? rounded[index]
      : formatDecimal(option.window, 0),
  )
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
 * (`crates/agent-server/src/server/tree/profiles.rs`), with `kept`, the
 * profile's own, shown where the row does not offer it:
 *
 * - Default, the user's limit on the model, with the window it gives now
 * - the Context row's options, the full window stored as its own size
 *
 * None for a model without a Context row.
 */
export function profileContextOptions(model: ModelInfo, kept: number | null): ProfileContextOption[] {
  const options = contextLimitOptions(model, kept)
  if (options.length === 0) {
    return []
  }
  const labels = contextOptionLabels(options)
  return [
    { label: t('models.default', { window: formatTokens(contextWindowInUse(model)) }), tokens: null },
    ...options.map((option, index) => ({ label: labels[index], tokens: option.window })),
  ]
}

/**
 * The profile's Context `tokens` as `model` offers it, null for Default: a
 * choice the window no longer offers is the full window, as for a session.
 */
export function profileContextChoice(model: ModelInfo, tokens: number | null): number | null {
  const offered = profileContextOptions(model, tokens).some((option) => option.tokens === tokens)
  return tokens === null || offered ? tokens : model.contextWindow
}
