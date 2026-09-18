import { z } from 'zod'

/**
 * A boolean query parameter (`web-api.md` § Query parameters): exactly `true`
 * or `false`, omitted meaning false; anything else is refused, never read as
 * one of them.
 */
export const booleanQuerySchema = z
  .stringbool({ truthy: ['true'], falsy: ['false'], case: 'sensitive' })
  .optional()
