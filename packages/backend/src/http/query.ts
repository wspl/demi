import { z } from 'zod'
import { LOG_READ_LINES, logCursorSchema } from '@demicodes/runner-protocol'

/**
 * A boolean query parameter (`web-api.md` § Query parameters): exactly `true`
 * or `false`, omitted meaning false; anything else is refused, never read as
 * one of them.
 */
export const booleanQuerySchema = z
  .stringbool({ truthy: ['true'], falsy: ['false'], case: 'sensitive' })
  .optional()

/**
 * The device log's query (`web-api.md` § Device log): `since` is the `next`
 * of an earlier answer, `limit` defaults to 200 and is at most what one
 * runner read returns, and `source` keeps the lines of one source.
 */
export const logQuerySchema = z.object({
  since: z.coerce.number().pipe(logCursorSchema).optional(),
  limit: z.coerce.number().int().min(1).max(LOG_READ_LINES).default(200),
  source: z.string().min(1).optional(),
})
