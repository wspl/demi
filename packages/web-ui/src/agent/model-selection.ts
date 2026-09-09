import type { ModelInfo, ProviderInfo } from '../transport/protocol'

export interface SelectedModel {
  providerId: string
  modelId: string
  model: ModelInfo
}

export type ComposerModelKind = 'ready' | 'unavailable' | 'none'

export interface ComposerModelState {
  kind: ComposerModelKind
  /** Catalog row for the last choice, even when that provider cannot send. */
  selected: SelectedModel | null
  providerId: string | null
  modelId: string | null
  label: string
}

/** Providers that are usable right now: available and with at least one catalog model. */
export function availableProviders(
  providers: readonly ProviderInfo[],
  models: Record<string, ModelInfo[]>
): ProviderInfo[] {
  return providers.filter(
    (provider) => provider.isAvailable && (models[provider.id]?.length ?? 0) > 0
  )
}

export function modelIsUsable(
  providers: readonly ProviderInfo[],
  models: Record<string, ModelInfo[]>,
  providerId: string,
  modelId: string,
): boolean {
  const provider = providers.find((item) => item.id === providerId)
  if (!provider?.isAvailable)
    return false
  return (models[providerId] ?? []).some((model) => model.id === modelId)
}

export function lookupSelectedModel(
  models: Record<string, ModelInfo[]>,
  providerId: string | null | undefined,
  modelId: string | null | undefined,
): SelectedModel | null {
  if (!providerId || !modelId)
    return null
  const model = (models[providerId] ?? []).find((candidate) => candidate.id === modelId)
  return model ? { providerId, modelId, model } : null
}

/**
 * What the composer can do with the last choice.
 * A stored id that is no longer usable stays selected so the chip can warn;
 * only a session with no last choice falls through to the first usable model.
 */
export function composerModel(
  providers: readonly ProviderInfo[],
  models: Record<string, ModelInfo[]>,
  providerId: string | null | undefined,
  modelId: string | null | undefined,
): ComposerModelState {
  const usable = availableProviders(providers, models)
  const last = lookupSelectedModel(models, providerId, modelId)

  if (providerId && modelId) {
    if (last && modelIsUsable(providers, models, providerId, modelId)) {
      return {
        kind: 'ready',
        selected: last,
        providerId,
        modelId,
        label: last.model.name
      }
    }
    const label = last?.model.name ?? modelId
    if (usable.length > 0) {
      return { kind: 'unavailable', selected: last, providerId, modelId, label }
    }
    return { kind: 'none', selected: last, providerId, modelId, label }
  }

  const first = usable[0]
  const model = first ? models[first.id]?.[0] : undefined
  if (first && model) {
    return {
      kind: 'ready',
      selected: { providerId: first.id, modelId: model.id, model },
      providerId: first.id,
      modelId: model.id,
      label: model.name,
    }
  }
  return {
    kind: 'none',
    selected: null,
    providerId: null,
    modelId: null,
    label: ''
  }
}

/** The model the chip can send with. Unusable last choices do not fall through. */
export function resolveSelectedModel(
  providers: readonly ProviderInfo[],
  models: Record<string, ModelInfo[]>,
  providerId: string | null | undefined,
  modelId: string | null | undefined,
): SelectedModel | null {
  const state = composerModel(providers, models, providerId, modelId)
  return state.kind === 'ready' ? state.selected : null
}
