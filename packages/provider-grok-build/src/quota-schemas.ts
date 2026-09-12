import { z } from 'zod'
import { quotaAmountSchema } from '@demicodes/provider'

const text = z.string().min(1)
const money = z.union([
  quotaAmountSchema,
  z.looseObject({ val: quotaAmountSchema }).transform((value) => value.val),
])
const timestamp = z.iso.datetime({ offset: true })
export const grokQuotaUserSchema = z.looseObject({
  subscriptionTier: text.nullish(),
  email: text.nullish(),
})
export const grokQuotaBillingSchema = z.looseObject({
  config: z.looseObject({
    currentPeriod: z.looseObject({ type: text.optional(), end: timestamp.nullish() }).nullish(),
    billingPeriodEnd: timestamp.nullish(),
    monthlyLimit: money.nullish(),
    used: money.nullish(),
    onDemandCap: money.nullish(),
    creditUsagePercent: quotaAmountSchema.nullish(),
  }).optional(),
})
export const grokRateLimitSchema = z.object({
  remaining: quotaAmountSchema.int().nullable(),
  limit: quotaAmountSchema.int().nullable(),
}).refine((value) => value.remaining === null || value.limit === null
  || value.remaining <= value.limit, { path: ['remaining'], message: 'Remaining exceeds limit' })
