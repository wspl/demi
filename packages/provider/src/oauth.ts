// Shapes every OAuth device-code login (RFC 8628) meets: the seconds a server
// states, and the one way a login response is turned into a typed value.
import { z } from 'zod'

/**
 * A duration in seconds as an OAuth server states it: RFC 8628 says a number,
 * some deployments send the digits as a string.
 */
export const oauthSecondsSchema = z.union([
  z.number(),
  z.string().trim().regex(/^\d+(\.\d+)?$/).transform(Number),
])

/**
 * RFC 8628 § 3.5: with no usable interval, the client polls every 5 seconds.
 */
export const DEFAULT_POLL_INTERVAL_SECONDS = 5

/**
 * How long to wait between polls. Anything unusable means "no preference",
 * which is the default interval rather than a failed login.
 */
export const pollIntervalSecondsSchema = oauthSecondsSchema
  .refine((seconds) => Number.isFinite(seconds) && seconds >= 0)
  .catch(DEFAULT_POLL_INTERVAL_SECONDS)

/** A device-code or token lifetime; an unusable one reads as absent. */
export const lifetimeSecondsSchema = oauthSecondsSchema
  .refine((seconds) => Number.isFinite(seconds) && seconds > 0)
  .optional()
  .catch(undefined)

/**
 * Decodes one login response against `schema`. A body that is not JSON, or
 * that does not match, is reported through `onMalformed`, which builds the
 * caller's own error from the schema's complaint.
 */
export async function decodeJsonResponse<T>(
  response: Response,
  schema: z.ZodType<T>,
  onMalformed: (problem: string) => Error,
): Promise<T> {
  const body: unknown = await response.json().catch(() => null)
  const decoded = schema.safeParse(body)
  if (!decoded.success)
    throw onMalformed(z.prettifyError(decoded.error))
  return decoded.data
}
