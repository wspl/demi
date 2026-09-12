import { z } from 'zod'

/** Visual restoration records are discarded as a whole when invalid. */
export const scrollPositionSchema = z.object({
  top: z.number().nonnegative(),
  anchor: z.string().nullable(),
  offset: z.number(),
})
export type ScrollPosition = z.infer<typeof scrollPositionSchema>

export const scrollAnchorSchema = z.object({
  blockId: z.string(),
  anchorIndex: z.number().int().nonnegative(),
  offsetPx: z.number(),
  scrollTop: z.number().nonnegative(),
})
export const persistedScrollStateSchema = z.object({
  anchor: scrollAnchorSchema,
  heightCache: z.map(z.string(), z.number().nonnegative()),
})
export type ScrollAnchor = z.infer<typeof scrollAnchorSchema>
export type PersistedScrollState = z.infer<typeof persistedScrollStateSchema>
