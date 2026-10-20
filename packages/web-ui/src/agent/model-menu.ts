import type { Component } from 'vue'
import type { ModelInfo, ModelServiceTier, ProviderInfo } from '../transport/protocol'
import type { TitleText } from '../ui/ui-text'
import { formatTokens } from '../ui/token-count'
import { contextLimitOptions, contextWindowInUse } from './context-limit'
import { fastServiceTier, isFastMode } from './fast-mode'
import {
  composerModel,
  resolveSelectedModel,
  type ModelSettings,
  type SelectedModel,
} from './model-selection'
import { buildReasoningState, reasoningOptionIndex, reasoningOptionLabel } from './reasoning'

/** What the model menu offers for `settings`: each row it shows, null for one it leaves out. */
export interface ModelMenuRows {
  /** The model the settings name, when the menu can send with it. */
  selected: SelectedModel | null
  /**
   * The choice of no model of its own at the top, chosen while the settings
   * are none: Parent's Model of a subagent profile, Automatic of the small
   * model.
   */
  standIn: { selected: boolean } | null
  /** Fast Mode, for a model with a Fast tier. */
  fast: { tier: ModelServiceTier; on: boolean } | null
  /** Reasoning, for a model that lists efforts: Off when it can turn thinking off, then each effort. */
  reasoning: {
    value: string
    options: { label: TitleText; effort: string; icon: Component; selected: boolean }[]
  } | null
  /** Context, for a model whose window offers limits: the full window, then each limit. */
  context: {
    value: string
    options: { label: string; window: number; tokens: number | null; selected: boolean }[]
  } | null
  /**
   * Model: the models of each usable provider, the chosen one marked, and
   * those of an incomplete entry too, which cannot be chosen: `unchoosable`
   * is the reason, shown on each dimmed row.
   */
  model: {
    value: string
    groups: {
      provider: ProviderInfo
      unchoosable: string | null
      models: { model: ModelInfo; selected: boolean }[]
    }[]
  }
}

/**
 * The rows of the model menu: one derivation for the composer, the profile editor and
 * the small model's menu, so the stand-in at the top, Parent's Model or Automatic, is
 * the only row another menu adds.
 */
export function modelMenuRows(
  providers: readonly ProviderInfo[],
  models: Record<string, ModelInfo[]>,
  settings: ModelSettings | null | undefined,
  standIn: boolean,
): ModelMenuRows {
  const standInChosen = standIn && !settings
  const selected = standInChosen
    ? null
    : resolveSelectedModel(providers, models, settings?.providerId, settings?.modelId)
  const model = selected?.model ?? null
  const effort = settings?.thinkingEffort ?? null

  const tier = fastServiceTier(model)
  const reasoning = buildReasoningState(model)
  const reasoningIndex = reasoning ? reasoningOptionIndex(reasoning, effort) : -1
  const contextOptions = model ? contextLimitOptions(model) : []
  const limit = model?.contextLimit ?? null

  return {
    selected,
    standIn: standIn ? { selected: standInChosen } : null,
    fast: tier ? { tier, on: isFastMode(model, settings?.serviceTierId) } : null,
    reasoning: reasoning
      ? {
          value: reasoningOptionLabel(reasoning, effort),
          options: reasoning.options.map((option, index) => ({ ...option, selected: index === reasoningIndex })),
        }
      : null,
    context: contextOptions.length
      ? {
          value: formatTokens(contextWindowInUse(model)),
          options: contextOptions.map((option) => ({
            ...option,
            label: formatTokens(option.window),
            selected: limit === option.tokens,
          })),
        }
      : null,
    model: {
      // A choice the menu cannot send with still names itself, as the composer's chip does.
      value: standInChosen
        ? ''
        : model?.name ?? composerModel(providers, models, settings?.providerId, settings?.modelId).label,
      groups: providers
        .filter((provider) => (provider.isAvailable || provider.setupReason !== undefined) && (models[provider.id]?.length ?? 0) > 0)
        .map((provider) => ({
          provider,
          unchoosable: provider.isAvailable ? null : provider.setupReason ?? null,
          models: (models[provider.id] ?? []).map((candidate) => ({
            model: candidate,
            selected: selected?.providerId === provider.id && selected.modelId === candidate.id,
          })),
        })),
    },
  }
}
