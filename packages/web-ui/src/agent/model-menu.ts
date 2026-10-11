import type { ModelInfo, ModelServiceTier, ProviderInfo } from '../transport/protocol'
import type { TitleText } from '../ui/ui-text'
import { formatTokens } from '../ui/token-count'
import { contextLimitOptions, contextWindowInUse } from './context-limit'
import { fastServiceTier, isFastMode } from './fast-mode'
import {
  availableProviders,
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
  /** Parent's Model at the top, offered to a subagent profile only; chosen while the settings are none. */
  parent: { selected: boolean } | null
  /** Fast Mode, for a model with a Fast tier. */
  fast: { tier: ModelServiceTier; on: boolean } | null
  /** Reasoning, for a model that lists efforts: Off when it can turn thinking off, then each effort. */
  reasoning: {
    value: string
    options: { label: TitleText; effort: string; selected: boolean }[]
  } | null
  /** Context, for a model whose window offers limits: the full window, then each limit. */
  context: {
    value: string
    options: { label: string; window: number; tokens: number | null; selected: boolean }[]
  } | null
  /** Model: the models of each usable provider, the chosen one marked. */
  model: {
    value: string
    groups: { provider: ProviderInfo; models: { model: ModelInfo; selected: boolean }[] }[]
  }
}

/**
 * The rows of the model menu (`product.md` § Settings: a profile's model is
 * chosen with the composer's menu, the same items plus Parent's Model): one
 * derivation for the composer and the profile editor, so Parent's Model is
 * the only row the profile's menu adds and none of the composer's is cut.
 */
export function modelMenuRows(
  providers: readonly ProviderInfo[],
  models: Record<string, ModelInfo[]>,
  settings: ModelSettings | null | undefined,
  parentModel: boolean,
): ModelMenuRows {
  const parentChosen = parentModel && !settings
  const selected = parentChosen
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
    parent: parentModel ? { selected: parentChosen } : null,
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
      value: parentChosen
        ? ''
        : model?.name ?? composerModel(providers, models, settings?.providerId, settings?.modelId).label,
      groups: availableProviders(providers, models).map((provider) => ({
        provider,
        models: (models[provider.id] ?? []).map((candidate) => ({
          model: candidate,
          selected: selected?.providerId === provider.id && selected.modelId === candidate.id,
        })),
      })),
    },
  }
}
