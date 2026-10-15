import { t } from './locales/text'
import { offersEffort, offersTier, type ModelSettings } from '../agent/model-selection'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import type { SentenceText } from '../ui/ui-text'

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
    return t('profile.providerGone')
  }
  const listed = (models[model.providerId] ?? []).find((candidate) => candidate.id === model.modelId)
  if (!listed) {
    return t('profile.modelGone', { model: model.modelId })
  }
  if (model.thinkingEffort !== null && !offersEffort(listed, model.thinkingEffort)) {
    return t('profile.effortGone', { model: listed.name, effort: model.thinkingEffort })
  }
  if (model.serviceTierId !== null && !offersTier(listed, model.serviceTierId)) {
    return t('profile.tierGone', { model: listed.name, tier: model.serviceTierId })
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
    return t('profile.parentModel')
  }
  const provider = providers.find((candidate) => candidate.id === model.providerId)
  const listed = (models[model.providerId] ?? []).find((candidate) => candidate.id === model.modelId)
  const name = listed?.name ?? model.modelId
  return provider ? `${name} · ${provider.label}` : name
}

/** The longest name a profile may have (`subagents.md` § Profiles). */
const PROFILE_NAME_MAX = 40

/**
 * What breaks the profile name rule in `name`, said as the fix, while it is
 * typed: 1 to 40 lowercase letters, digits and hyphens, starting with a
 * letter, and not `default`, which the inherit profile has. Null for a name
 * that keeps the rule, and for an empty one, which only cannot be saved yet.
 */
export function profileNameProblem(name: string): SentenceText | null {
  if (name === '') {
    return null
  }
  if (!/^[a-z0-9-]*$/.test(name)) {
    return t('profile.nameCharacters')
  }
  if (!/^[a-z]/.test(name)) {
    return t('profile.nameStart')
  }
  if (name.length > PROFILE_NAME_MAX) {
    return t('profile.nameLength', { max: PROFILE_NAME_MAX })
  }
  if (name === 'default') {
    return t('profile.nameDefault')
  }
  return null
}
