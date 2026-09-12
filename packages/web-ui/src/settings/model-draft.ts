import { configuredModelSchema } from '@demicodes/product-contracts'
import type { SettingsModelDraft } from './types'

/** Projects form names and dot-prefixed extensions into the shared model contract. */
export function readConfiguredModelDraft(model: SettingsModelDraft) {
  return configuredModelSchema.safeParse({
    id: model.id,
    displayName: model.name.trim() || model.id,
    contextWindow: model.contextWindow,
    outputLimit: model.outputLimit,
    thinkingEfforts: model.efforts,
    acceptedExtensions:
      model.extensions?.map((extension) => extension.replace(/^\./, '')) ??
      null,
    fastTier: model.fastTier,
  })
}
