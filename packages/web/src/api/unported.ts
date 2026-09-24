import { z } from 'zod'
import { productStateSchema as portedProductStateSchema } from './generated/web-api'

// The REST bodies of routes the Rust backend does not serve yet: exposes, and
// the part of the product state the exposes fill. Their web-api types do not
// exist yet, so these follow `web-api.md`; each goes when its generated type
// does, and nothing else in the page declares a REST shape.

const time = z.iso.datetime({ precision: 3 })

/** An expose record (`web-api.md` § Exposes). */
export const exposeSchema = z.object({
  id: z.string().min(1),
  deviceId: z.string().min(1),
  address: z.string().min(1),
  url: z.string(),
  createdAt: time,
  expiresAt: time,
})
export type Expose = z.infer<typeof exposeSchema>

export const exposeAnswerSchema = z.object({ expose: exposeSchema })

/**
 * `GET /state` with the exposes, which the Rust backend does not send yet:
 * without them there are no exposes to show.
 */
export const productStateSchema = portedProductStateSchema.extend({
  exposes: z.array(exposeSchema).optional(),
  exposeDomain: z.string().nullable().optional(),
})
export type ProductState = z.infer<typeof productStateSchema>
