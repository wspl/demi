// Mechanical conversions from a provider's catalog (`ProviderModel`) into the
// `ModelSelection` the agent runtime consumes. UIs and embedders share these so
// nobody hand-rolls a `Model` literal from `listModels()` output.
import type {
  FileExtension,
  ModelSelection,
  ThinkingCapability,
  ThinkingConfig,
  ThinkingSummary,
} from '@demicodes/core'
import type { ProviderModel, ProviderModelList } from './types'

/** Stamps `providerId` onto a model catalog and every model in it. */
export function withProviderId(list: ProviderModelList, providerId: string): ProviderModelList {
  return {
    ...list,
    providerId,
    models: list.models.map((model) => ({ ...model, providerId })),
  }
}

/** Attachment file types a model is offered when it reports attachment support. */
export const DEFAULT_ATTACHMENT_EXTENSIONS: readonly FileExtension[] = [
  'png',
  'jpg',
  'jpeg',
  'gif',
  'webp',
  'pdf',
]

/** Derive the thinking capabilities a UI can offer from a catalog model. */
export function thinkingCapabilitiesFromProviderModel(model: ProviderModel | null): ThinkingCapability[] {
  if (!model) return []
  if (model.supportsReasoning === false) return [{ type: 'disabled' }]
  if (!model.supportedThinkingEfforts || model.supportedThinkingEfforts.length === 0) return []
  const summaries: ThinkingSummary[] = ['auto', 'concise', 'detailed', 'off', 'on']
  return [
    {
      type: 'effort',
      efforts: model.supportedThinkingEfforts,
      defaultEffort: null,
      summaries,
      defaultSummary: null,
    },
  ]
}

export interface ModelSelectionFromCatalogOptions {
  /** Override the model id (defaults to `model.id`); useful before the catalog has loaded. */
  modelId?: string
  /** Active thinking configuration, if any. */
  thinking?: ThinkingConfig | null
  /** Active service tier, if the model exposes tiers. */
  serviceTierId?: string | null
  /** Attachment types to accept when the model supports attachments. */
  acceptedExtensions?: readonly FileExtension[]
  /** Display name to use when the catalog entry is absent. */
  fallbackName?: string
}

/**
 * Build a `ModelSelection` from a catalog `ProviderModel`. Pass `model: null`
 * (with `modelId`) to construct a selection before the catalog has been fetched.
 */
export function modelSelectionFromCatalog(
  providerId: string,
  model: ProviderModel | null,
  options: ModelSelectionFromCatalogOptions = {},
): ModelSelection {
  const modelId = options.modelId ?? model?.id ?? ''
  const accepted = options.acceptedExtensions ?? DEFAULT_ATTACHMENT_EXTENSIONS
  return {
    providerId,
    model: {
      id: modelId,
      name: model?.displayName ?? options.fallbackName ?? modelId,
      contextWindow: model?.contextWindow ?? 0,
      inputLimit: null,
      thinking: thinkingCapabilitiesFromProviderModel(model),
      acceptedExtensions: model?.supportsAttachments ? [...accepted] : [],
    },
    thinking: options.thinking ?? null,
    serviceTierId: options.serviceTierId ?? null,
  }
}
