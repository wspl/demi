import type { ConfiguredModel } from '@demicodes/product-contracts'
import { modelSelectionFromCatalog, type ProviderModel } from '@demicodes/provider'
import type { ModelSelection } from '@demicodes/core'

/** Shared factory options for the three API families. */
export function runtimeModelOptions(model: ConfiguredModel) {
  return {
    id: model.id,
    displayName: model.displayName,
    contextWindow: model.contextWindow,
    outputLimit: model.outputLimit,
    supportsTools: true,
    supportsAttachments: model.acceptedExtensions === null
      ? null
      : model.acceptedExtensions.length > 0,
    supportsReasoning: model.thinkingEfforts.length > 0,
    supportedThinkingEfforts: model.thinkingEfforts,
    defaultThinkingEffort: model.thinkingEfforts[0] ?? null,
    serviceTiers: model.fastTier
      ? [{ id: model.fastTier, label: 'Fast', fast: true }]
      : [],
  }
}

export function configuredCatalogModel(
  providerId: string,
  model: ConfiguredModel
): ProviderModel {
  return {
    ...runtimeModelOptions(model),
    providerId,
    acceptedExtensions: model.acceptedExtensions,
    sourceFetchedAt: '1970-01-01T00:00:00.000Z',
    stale: false,
  }
}

/**
 * Server-owned metadata wins over a browser's old catalog snapshot;
 * user-selected thinking/tier remain explicit.
 */
export function applyConfiguredModel(
  providerId: string,
  model: ConfiguredModel,
  selection: ModelSelection
): ModelSelection {
  return modelSelectionFromCatalog(
    providerId,
    configuredCatalogModel(providerId, model),
    {
      acceptedExtensions: model.acceptedExtensions,
      thinking: selection.thinking,
      serviceTierId: selection.serviceTierId,
    }
  )
}
