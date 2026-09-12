import { z } from 'zod'
import { DEFAULT_ATTACHMENT_EXTENSIONS, type ProviderModel } from '@demicodes/provider'
import { VIDEO_FILE_EXTENSIONS } from '@demicodes/core'

const modelSchema = z.object({
  providerId: z.string().min(1),
  id: z.string().min(1),
  displayName: z.string(),
  description: z.string().optional(),
  contextWindow: z.number().nullable(),
  outputLimit: z.number().nullable(),
  supportsTools: z.boolean().nullable(),
  supportsAttachments: z.boolean().nullable(),
  acceptedExtensions: z.array(z.enum([
    ...DEFAULT_ATTACHMENT_EXTENSIONS,
    ...VIDEO_FILE_EXTENSIONS,
  ])).nullable().optional(),
  supportsVideo: z.boolean().nullable().optional(),
  supportsReasoning: z.boolean().nullable(),
  supportedThinkingEfforts: z.array(z.string()).nullable(),
  defaultThinkingEffort: z.string().nullable(),
  canDisableThinking: z.boolean().nullable().optional(),
  serviceTiers: z.array(z.object({
    id: z.string(), label: z.string(), description: z.string().optional(), fast: z.boolean(),
  })).nullable().optional(),
  defaultServiceTierId: z.string().nullable().optional(),
  cost: z.object({
    input: z.number().nullable(), output: z.number().nullable(),
    cacheRead: z.number().nullable(), cacheWrite: z.number().nullable(),
  }).optional(),
  sourceFetchedAt: z.iso.datetime({ offset: true }),
  stale: z.boolean(),
}) satisfies z.ZodType<ProviderModel>

export const catalogSnapshotSchema = z.object({
  models: z.array(modelSchema),
  sourceFetchedAt: z.iso.datetime({ offset: true }),
  stale: z.boolean(),
  warnings: z.array(z.string()),
})
export type CatalogSnapshot = z.infer<typeof catalogSnapshotSchema>
export const modelCatalogRecordSchema = z.object({
  key: z.string().min(1),
  checkedAt: z.number().int().nonnegative(),
  catalog: catalogSnapshotSchema,
})
export type ModelCatalogRecord = z.infer<typeof modelCatalogRecordSchema>
