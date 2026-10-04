import { offersEffort, offersTier, type ModelSettings } from '../agent/model-selection'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'

/**
 * What of a profile's model settings the catalog the page holds no longer
 * offers (`subagents.md` § An unavailable profile): its provider entry, its
 * model, its effort or its tier; null while the profile is available, or
 * inherits its parent's model. The page derives it and never stores it.
 */
export function profileMissing(
  model: ModelSettings | null,
  providers: readonly ProviderInfo[],
  models: Record<string, ModelInfo[]>,
): string | null {
  if (!model) {
    return null
  }
  if (!providers.some((provider) => provider.id === model.providerId)) {
    return 'Its provider is gone. Choose another model.'
  }
  const listed = (models[model.providerId] ?? []).find((candidate) => candidate.id === model.modelId)
  if (!listed) {
    return `The model ${model.modelId} is gone. Choose another model.`
  }
  if (model.thinkingEffort !== null && !offersEffort(listed, model.thinkingEffort)) {
    return `${listed.name} no longer offers the effort ${model.thinkingEffort}. Choose another effort.`
  }
  if (model.serviceTierId !== null && !offersTier(listed, model.serviceTierId)) {
    return `${listed.name} no longer offers the tier ${model.serviceTierId}. Choose another tier.`
  }
  return null
}

/** The model a profile names, as its row reads it: the model's name and its provider's. */
export function profileModelLabel(
  model: ModelSettings | null,
  providers: readonly ProviderInfo[],
  models: Record<string, ModelInfo[]>,
): string {
  if (!model) {
    return 'Parent’s model'
  }
  const provider = providers.find((candidate) => candidate.id === model.providerId)
  const listed = (models[model.providerId] ?? []).find((candidate) => candidate.id === model.modelId)
  const name = listed?.name ?? model.modelId
  return provider ? `${name} · ${provider.label}` : name
}
