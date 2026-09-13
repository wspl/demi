/**
 * How Demi describes a vendor's wire payloads: the two-step decode of a
 * payload tagged by `type`, and the field shapes every vendor module repeats.
 * These live here rather than in `@demicodes/utils`, which is zero-dependency
 * and cannot hold a zod helper.
 */
import { z } from 'zod'

/**
 * A token count as vendors report it: a number, or absent (also spelled
 * `null`).
 */
export const tokenCountSchema = z.number().nullable().optional()

/**
 * A field Demi only reports back to the user, such as a vendor error's code or
 * message: anything but a string reads as absent, so a malformed error payload
 * still surfaces as that error instead of as a protocol failure.
 */
export const reportedStringSchema = z.string().optional().catch(undefined)

/**
 * A union of vendor payloads keyed by their `type` tag: read the tag first,
 * then validate the payload with the schema registered for that tag.
 *
 * An unregistered tag decodes to `null` — a vendor adding an event or item
 * type must not break a stream Demi otherwise understands. A registered tag
 * whose payload is malformed is a parse error naming the offending field,
 * because that is the vendor breaking a contract Demi reads.
 *
 * Give each branch a `z.literal(<tag>)` `type` field so the decoded value
 * narrows on `type` the way a discriminated union does.
 */
export function taggedUnion<T extends z.ZodType>(
  branches: Record<string, T>
): z.ZodType<z.output<T> | null, unknown> {
  const known = new Map(Object.entries(branches))
  return z.looseObject({ type: z.string() }).transform((payload, ctx) => {
    const branch = known.get(payload.type)
    if (!branch)
      return null
    const result = branch.safeParse(payload)
    if (result.success)
      return result.data
    // Re-raise the branch's issues here so their paths name the field.
    for (const issue of result.error.issues) ctx.addIssue({ ...issue })
    return z.NEVER
  })
}
