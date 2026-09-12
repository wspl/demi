import { z } from 'zod'

const id = z.string().min(1).regex(/^\S+$/)
export const grokCatalogModelSchema = z.looseObject({
  id,
  name: z.string().min(1).optional(),
  description: z.string().optional(),
  context_window: z.number().int().positive().nullish(),
  input_modalities: z.array(id).optional(),
  supports_reasoning_effort: z.boolean().optional(),
  reasoning_efforts: z.array(z.looseObject({ id, default: z.boolean().optional() })).optional(),
  reasoning_effort: id.optional(),
}).superRefine((model, context) => {
  const efforts = model.reasoning_efforts
  if (model.supports_reasoning_effort === false
    && (efforts?.length || model.reasoning_effort !== undefined)) {
    context.addIssue({ code: 'custom', path: ['supports_reasoning_effort'], message: 'Conflicting capability' })
  }
  if (!efforts) {
    return
  }
  const defaults = efforts.filter((effort) => effort.default)
  if (defaults.length > 1 || (model.reasoning_effort !== undefined
    && defaults[0] && defaults[0].id !== model.reasoning_effort)) {
    context.addIssue({ code: 'custom', path: ['reasoning_effort'], message: 'Conflicting defaults' })
  }
  if (new Set(efforts.map((effort) => effort.id)).size !== efforts.length) {
    context.addIssue({ code: 'custom', path: ['reasoning_efforts'], message: 'Duplicate effort IDs' })
  }
  if (model.reasoning_effort !== undefined
    && !efforts.some((effort) => effort.id === model.reasoning_effort)) {
    context.addIssue({ code: 'custom', path: ['reasoning_effort'], message: 'Default effort is not offered' })
  }
})
export const grokModelsResponseSchema = z.looseObject({ data: z.array(grokCatalogModelSchema) })
  .refine((response) => new Set(response.data.map((model) => model.id)).size === response.data.length,
    { path: ['data'], message: 'Duplicate model IDs' })
export type GrokModelsResponse = z.infer<typeof grokModelsResponseSchema>
