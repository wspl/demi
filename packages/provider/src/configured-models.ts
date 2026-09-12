import { z } from 'zod'
import { parseProviderData } from './validation'
import type { ProviderModel, ProviderModelList } from './types'

const id = z.string().min(1).regex(/^\S+$/)
const label = z.string().min(1).regex(/\S/)
export const configuredModelSchema = z.strictObject({
  id,
  displayName: label.optional(),
  description: z.string().optional(),
  contextWindow: z.number().int().positive(),
  outputLimit: z.number().int().positive().nullish(),
  supportsTools: z.boolean().nullish(),
  supportsAttachments: z.boolean().nullish(),
  supportsVideo: z.boolean().nullish(),
  supportsReasoning: z.boolean().nullish(),
  supportedThinkingEfforts: z.array(id).nullish(),
  defaultThinkingEffort: id.nullish(),
  canDisableThinking: z.boolean().nullish(),
  serviceTiers: z.array(z.strictObject({
    id,
    label,
    description: z.string().optional(),
    fast: z.boolean(),
  })).nullish(),
  defaultServiceTierId: id.nullish(),
})
const catalogOptionsSchema = z.strictObject({
  providerId: id,
  defaultModelId: id.nullish(),
  sourceFetchedAt: z.iso.datetime({ offset: true }).optional(),
  stale: z.boolean().optional(),
})
const configuredCatalogSchema = catalogOptionsSchema.extend({ models: z.array(configuredModelSchema) })
  .superRefine((catalog, context) => {
    const ids = new Set(catalog.models.map((model) => model.id))
    if (ids.size !== catalog.models.length) {
      context.addIssue({ code: 'custom', path: ['models'], message: 'Duplicate model IDs' })
    }
    if (catalog.defaultModelId != null && !ids.has(catalog.defaultModelId)) {
      context.addIssue({ code: 'custom', path: ['defaultModelId'], message: 'Default model is not in the catalog' })
    }
  })

export type ConfiguredModelOptions = z.infer<typeof configuredModelSchema>
export type ConfiguredCatalogOptions = z.infer<typeof catalogOptionsSchema>

/** Static built-in catalogs have no remote fetch date. */
export const STATIC_CATALOG_SOURCE_DATE = '1970-01-01T00:00:00.000Z'

/** Shared validation and projection for caller-configured API model catalogs. */
export function modelListFromConfiguredModels(
  models: ConfiguredModelOptions[],
  options: ConfiguredCatalogOptions,
): ProviderModelList {
  const catalog = parseProviderData(configuredCatalogSchema, { ...options, models }, 'Configured model catalog')
  const sourceFetchedAt = catalog.sourceFetchedAt ?? new Date().toISOString()
  const stale = catalog.stale ?? false
  const mapped: ProviderModel[] = catalog.models.map((model) => ({
    providerId: catalog.providerId,
    id: model.id,
    displayName: model.displayName ?? model.id,
    description: model.description,
    contextWindow: model.contextWindow,
    outputLimit: model.outputLimit ?? null,
    supportsTools: model.supportsTools ?? null,
    supportsAttachments: model.supportsAttachments ?? null,
    supportsVideo: model.supportsVideo ?? null,
    supportsReasoning: model.supportsReasoning ?? null,
    supportedThinkingEfforts: model.supportedThinkingEfforts ?? null,
    defaultThinkingEffort: model.defaultThinkingEffort ?? null,
    canDisableThinking: model.canDisableThinking ?? null,
    serviceTiers: model.serviceTiers ?? null,
    defaultServiceTierId: model.defaultServiceTierId ?? null,
    sourceFetchedAt,
    stale,
  }))
  return {
    providerId: catalog.providerId,
    models: mapped,
    defaultModelId: catalog.defaultModelId === undefined ? mapped[0]?.id ?? null : catalog.defaultModelId,
    warnings: [],
    sourceFetchedAt,
    stale,
  }
}
