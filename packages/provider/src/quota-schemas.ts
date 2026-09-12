import { z } from 'zod'

export const quotaAmountSchema = z.number().nonnegative()
export const quotaEpochSecondsSchema = quotaAmountSchema.max(8_640_000_000_000)

/** Quota wire timestamps use epoch seconds or an ISO date with a timezone. */
export const quotaResetSchema = z.union([
  quotaEpochSecondsSchema,
  z.string().regex(/^\d+(?:\.\d+)?$/).transform(Number).pipe(quotaEpochSecondsSchema),
]).transform((seconds) => new Date(seconds * 1000).toISOString())
  .or(z.iso.datetime({ offset: true }))
