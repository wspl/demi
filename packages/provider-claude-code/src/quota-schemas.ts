import { z } from 'zod'
import { quotaAmountSchema, quotaResetSchema } from '@demicodes/provider'

const text = z.string().min(1)
export const claudeQuotaWindowSchema = z.looseObject({
  utilization: quotaAmountSchema.nullish(),
  used_percentage: quotaAmountSchema.nullish(),
  resets_at: quotaResetSchema.nullish(),
})
export const claudeQuotaPayloadSchema = z.looseObject({
  five_hour: claudeQuotaWindowSchema.nullish(),
  seven_day: claudeQuotaWindowSchema.nullish(),
  seven_day_sonnet: claudeQuotaWindowSchema.nullish(),
  seven_day_opus: claudeQuotaWindowSchema.nullish(),
  limits: z.array(z.looseObject({
    kind: text,
    percent: quotaAmountSchema.nullish(),
    resets_at: quotaResetSchema.nullish(),
    severity: z.enum(['normal', 'warning', 'critical']).nullish(),
    scope: z.looseObject({
      model: z.looseObject({ display_name: text.optional() }).nullish(),
    }).nullish(),
  })).nullish(),
})
export const claudeQuotaEnvelopeSchema = z.looseObject({
  rate_limits: claudeQuotaPayloadSchema.nullish(),
  message: z.looseObject({ rate_limits: claudeQuotaPayloadSchema.nullish() }).optional(),
})
export type ClaudeQuotaPayload = z.infer<typeof claudeQuotaPayloadSchema>
export type ClaudeQuotaWindow = z.infer<typeof claudeQuotaWindowSchema>
