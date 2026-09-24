import { z } from 'zod'
import { cloudStatusSchema, productStateSchema as portedProductStateSchema } from './generated/web-api'

// The REST bodies of routes the Rust backend does not serve yet: exposes and
// a process provider's command-line tool, and the parts of the product state
// the exposes and the Cloud fill. Their web-api types do not exist yet, so
// these follow `web-api.md`; each goes when its generated type does, and
// nothing else in the page declares a REST shape.

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
 * `GET /state` with the parts the Rust backend does not send yet: without
 * them there are no exposes and no Cloud state to show.
 */
export const productStateSchema = portedProductStateSchema.extend({
  cloud: cloudStatusSchema.optional(),
  exposes: z.array(exposeSchema).optional(),
  exposeDomain: z.string().nullable().optional(),
})
export type ProductState = z.infer<typeof productStateSchema>

/** `GET /providers/:id/cli` (`web-api.md` § Model configuration and provider inspection). */
export const providerCliSchema = z.object({
  newest: z.union([z.object({ version: z.string() }), z.object({ error: z.string() })]),
  install: z.discriminatedUnion('state', [
    z.object({ state: z.literal('installing') }),
    z.object({ state: z.literal('installed'), path: z.string() }),
    z.object({ state: z.literal('failed'), message: z.string() }),
  ]).nullable(),
  machines: z.array(z.object({
    deviceId: z.string(),
    name: z.string(),
    versions: z.array(z.string()).nullable(),
  })),
})
