import { z } from 'zod'
import { quotaAmountSchema, quotaEpochSecondsSchema } from '@demicodes/provider'

export const codexQuotaWindowSchema = z.object({
  usedPercent: quotaAmountSchema.nullable(),
  windowMinutes: quotaAmountSchema.nullable(),
  resetSeconds: quotaEpochSecondsSchema.nullable(),
})
