import { isFastMode } from '../agent/fast-mode'
import { offersEffort, type ModelSettings } from '../agent/model-selection'
import { buildReasoningState, reasoningOptionLabel } from '../agent/reasoning'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import { profileModelLabel } from './subagent-profiles'

/** What a settings row's model menu trigger reads, as the composer's chip does. */
export interface ModelChoiceLabel {
  /** The model with its provider. */
  name: string
  /** Its effort, a step quieter; empty for a model without efforts. */
  effort: string
  /** Whether Fast is on, which the bolt says. */
  fast: boolean
}

/**
 * The trigger's label for the model settings `settings`, which a subagent profile
 * and the small model hold: the model with its provider, then its effort, then
 * whether Fast is on.
 *
 * An effort the model no longer offers reads as itself, never as the first one a
 * menu would show.
 */
export function modelChoiceLabel(
  settings: ModelSettings,
  providers: readonly ProviderInfo[],
  catalogModels: Record<string, ModelInfo[]>,
): ModelChoiceLabel {
  const listed = (catalogModels[settings.providerId] ?? []).find((model) => model.id === settings.modelId)
  const effort = settings.thinkingEffort
  const reasoning = buildReasoningState(listed)
  return {
    name: profileModelLabel(settings, providers, catalogModels),
    effort: listed && effort !== null && !offersEffort(listed, effort)
      ? effort
      : reasoning ? reasoningOptionLabel(reasoning, effort) : '',
    fast: isFastMode(listed, settings.serviceTierId),
  }
}
