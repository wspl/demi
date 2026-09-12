import { z } from 'zod'
import { FILE_EXTENSIONS } from '@demicodes/core'

/**
 * Explicit metadata for a manually configured API model. Unknown capabilities
 * remain null.
 */
export const configuredModelSchema = z
  .strictObject({
    id: z.string().trim().min(1).max(256),
    displayName: z.string().trim().min(1).max(256),
    contextWindow: z.number().int().positive(),
    outputLimit: z.number().int().positive().nullable(),
    thinkingEfforts: z.array(z.string().min(1).max(64)).max(32),
    acceptedExtensions: z.array(z.enum(FILE_EXTENSIONS)).max(64).nullable(),
    fastTier: z.string().min(1).max(64).nullable(),
  })
  .refine(
    (model) =>
      model.outputLimit === null || model.outputLimit <= model.contextWindow,
    { message: 'Output limit exceeds context window' },
  )
export const configuredModelsSchema = z
  .array(configuredModelSchema)
  .min(1)
  .max(1000)
  .refine(
    (models) => new Set(models.map((model) => model.id)).size === models.length,
    { message: 'Model IDs must be unique' },
  )
export type ConfiguredModel = z.infer<typeof configuredModelSchema>
