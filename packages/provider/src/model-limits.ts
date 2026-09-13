import { z } from 'zod'

/**
 * The token limits a catalog entry must state: whole positive counts, with no
 * output limit spelled as null. Everything else about a model is descriptive,
 * so only these two carry a constraint worth checking. Every provider that
 * builds a `ProviderModelList` from its own catalog checks them with this
 * schema.
 */
export const modelLimitsSchema = z.object({
  contextWindow: z.number().int().positive(),
  outputLimit: z.number().int().positive().nullish(),
})
