import type { ModelInfo, ProviderInfo } from '../transport/protocol'

export interface SelectedModel {
  providerId: string
  modelId: string
  model: ModelInfo
}

/** Providers that are usable right now: available and with at least one catalog model. */
export function availableProviders(providers: readonly ProviderInfo[], models: Record<string, ModelInfo[]>): ProviderInfo[] {
  return providers.filter((provider) => provider.isAvailable && (models[provider.id]?.length ?? 0) > 0)
}

/**
 * The model the chip shows: the session's choice when the catalog has it, else the first
 * model of the first usable provider, else nothing (the catalog is empty).
 */
export function resolveSelectedModel(
  providers: readonly ProviderInfo[],
  models: Record<string, ModelInfo[]>,
  providerId: string | null | undefined,
  modelId: string | null | undefined,
): SelectedModel | null {
  if (providerId && modelId) {
    const model = (models[providerId] ?? []).find((candidate) => candidate.id === modelId)
    if (model) return { providerId, modelId, model }
  }
  for (const provider of availableProviders(providers, models)) {
    const model = models[provider.id]?.[0]
    if (model) return { providerId: provider.id, modelId: model.id, model }
  }
  return null
}
