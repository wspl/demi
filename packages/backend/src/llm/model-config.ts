import { z } from 'zod'
import { DEFAULT_ATTACHMENT_EXTENSIONS, modelSelectionFromCatalog, type ProviderModel } from '@demicodes/provider'
import { VIDEO_FILE_EXTENSIONS, type ModelSelection } from '@demicodes/core'

/** Explicit metadata for a manually configured API model. Unknown capabilities remain null. */
export const configuredModelSchema = z.strictObject({
  id: z.string().trim().min(1).max(256),
  displayName: z.string().trim().min(1).max(256),
  contextWindow: z.number().int().positive(),
  outputLimit: z.number().int().positive().nullable(),
  thinkingEfforts: z.array(z.string().min(1).max(64)).max(32),
  acceptedExtensions: z.array(z.enum([...DEFAULT_ATTACHMENT_EXTENSIONS, ...VIDEO_FILE_EXTENSIONS])).max(64).nullable(),
  fastTier: z.string().min(1).max(64).nullable(),
}).refine(model => model.outputLimit === null || model.outputLimit <= model.contextWindow, { message: 'Output limit exceeds context window' })
export const configuredModelsSchema = z.array(configuredModelSchema).min(1).max(1000)
  .refine(models => new Set(models.map(model => model.id)).size === models.length, { message: 'Model IDs must be unique' })
export type ConfiguredModel = z.infer<typeof configuredModelSchema>

/** Shared factory options for the three API families. */
export function runtimeModelOptions(model: ConfiguredModel) {
  return {
    id: model.id,
    displayName: model.displayName,
    contextWindow: model.contextWindow,
    outputLimit: model.outputLimit,
    supportsTools: true,
    supportsAttachments: model.acceptedExtensions === null ? null : model.acceptedExtensions.length > 0,
    supportsReasoning: model.thinkingEfforts.length > 0,
    supportedThinkingEfforts: model.thinkingEfforts,
    defaultThinkingEffort: model.thinkingEfforts[0] ?? null,
    serviceTiers: model.fastTier ? [{ id: model.fastTier, label: 'Fast', fast: true }] : [],
  }
}

export function configuredCatalogModel(providerId: string, model: ConfiguredModel): ProviderModel & { acceptedExtensions: string[] | null } {
  return {
    ...runtimeModelOptions(model), providerId,
    acceptedExtensions: model.acceptedExtensions,
    sourceFetchedAt: '1970-01-01T00:00:00.000Z', stale: false,
  }
}

/** Server-owned metadata wins over a browser's old catalog snapshot; user-selected thinking/tier remain explicit. */
export function applyConfiguredModel(providerId: string, model: ConfiguredModel, selection: ModelSelection): ModelSelection {
  return modelSelectionFromCatalog(providerId, configuredCatalogModel(providerId, model), {
    acceptedExtensions: model.acceptedExtensions,
    thinking: selection.thinking,
    serviceTierId: selection.serviceTierId,
  })
}
