import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import { z } from 'zod'
import { fastServiceTier, isFastMode } from './fast-mode'

/** The thinking effort that turns thinking off. */
export const THINKING_OFF = 'disabled'

/**
 * A conversation's model settings (`models.md` § A conversation's model
 * settings): the provider entry and model, the thinking effort, and the
 * service tier. A conversation with a record shows the record's; a new one
 * keeps its own until its first send writes them there. An empty provider
 * and model mean none is chosen yet.
 */
export const modelSettingsSchema = z.object({
  providerId: z.string(),
  modelId: z.string(),
  /**
   * An effort the model lists, or `disabled` for thinking off; null only for
   * a model that lists no efforts, and in a draft before a model is chosen.
   */
  thinkingEffort: z.string().nullable(),
  /** A tier the model lists, or null for the vendor's default. */
  serviceTierId: z.string().nullable(),
})

export type ModelSettings = z.infer<typeof modelSettingsSchema>

/**
 * One change of model settings, naming the parts it changes. A switch names
 * the model, with the effort and the tier it keeps; for a part it leaves out
 * the backend takes the new model's first effort and the vendor's default
 * tier. A change without a model sets the parts it names and keeps the
 * others.
 */
export interface ModelSettingsChange {
  model?: { providerId: string; modelId: string }
  thinkingEffort?: string
  serviceTierId?: string | null
}

/** A new conversation starts with the user's last choice, or with none. */
export function initialModelSettings(previous?: ModelSettings): ModelSettings {
  return previous ? { ...previous } : {
    providerId: '',
    modelId: '',
    thinkingEffort: null,
    serviceTierId: null,
  }
}

/**
 * `settings` with `change` made, as the backend makes it of a record's. A
 * switch that names no effort holds none here, which the page shows and a
 * first send writes as the model's first effort.
 */
export function applyModelChange(settings: ModelSettings, change: ModelSettingsChange): ModelSettings {
  if (change.model) {
    return {
      providerId: change.model.providerId,
      modelId: change.model.modelId,
      thinkingEffort: change.thinkingEffort ?? null,
      serviceTierId: change.serviceTierId ?? null,
    }
  }
  return {
    ...settings,
    ...(change.thinkingEffort === undefined ? {} : { thinkingEffort: change.thinkingEffort }),
    ...(change.serviceTierId === undefined ? {} : { serviceTierId: change.serviceTierId }),
  }
}

/**
 * The effort a model's settings hold when no change names one: the first the
 * model lists (`models.md` § A conversation's model settings), or null when
 * it lists none.
 */
export function firstEffort(model: ModelInfo): string | null {
  return model.reasoning?.efforts[0] ?? null
}

/** Whether `model` offers the effort `effort`: one it lists, or thinking off when it can turn thinking off. */
export function offersEffort(model: ModelInfo, effort: string): boolean {
  const reasoning = model.reasoning
  if (!reasoning) {
    return false
  }
  return effort === THINKING_OFF ? reasoning.canDisable : reasoning.efforts.includes(effort)
}

/** Whether `model` offers the service tier `tier`. */
export function offersTier(model: ModelInfo, tier: string): boolean {
  return model.serviceTiers?.some((listed) => listed.id === tier) ?? false
}

/**
 * The change a switch to the model `next` of the entry `providerId` makes of
 * `settings`, whose model is `current` (`models.md` § A conversation's model
 * settings): it keeps the effort when the new model offers it, else names the
 * new model's first effort, and keeps Fast when Fast is on and the new model
 * has a Fast tier of its own; the vendor's default tier stands otherwise.
 */
export function modelSwitch(
  settings: Pick<ModelSettings, 'thinkingEffort' | 'serviceTierId'> | null | undefined,
  current: ModelInfo | null | undefined,
  providerId: string,
  next: ModelInfo,
): ModelSettingsChange {
  const change: ModelSettingsChange = { model: { providerId, modelId: next.id } }
  const effort = offeredEffort(settings?.thinkingEffort ?? null, next)
  if (effort !== null) {
    change.thinkingEffort = effort
  }
  const fast = fastServiceTier(next)
  if (fast && isFastMode(current, settings?.serviceTierId)) {
    change.serviceTierId = fast.id
  }
  return change
}

/**
 * The settings a new conversation's first send writes to its record: each
 * part its model still offers; for one it no longer offers, the model's first
 * effort and the vendor's default tier.
 */
export function offeredSettings(settings: ModelSettings, model: ModelInfo): ModelSettings {
  const tier = settings.serviceTierId
  return {
    ...settings,
    thinkingEffort: offeredEffort(settings.thinkingEffort, model),
    serviceTierId: tier !== null && offersTier(model, tier) ? tier : null,
  }
}

/** `effort` when `model` offers it, else the model's first effort. */
function offeredEffort(effort: string | null, model: ModelInfo): string | null {
  return effort !== null && offersEffort(model, effort) ? effort : firstEffort(model)
}

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
