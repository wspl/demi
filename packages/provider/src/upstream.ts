/**
 * Reading what a stored vendor failure says (`ProviderErrorDiagnostics.upstream`,
 * `docs/provider-errors-and-retries.md`). The record keeps the vendor's own
 * payload; the facts a reader needs are read out of it here, where the vendor
 * formats are known, and never stored beside it.
 */
import { z } from 'zod'
import { retryAfterMsFromHeader } from './http'

const limitFieldsSchema = z.looseObject({
  /** Epoch seconds at which a usage limit lifts. */
  resets_at: z.number().optional().catch(undefined),
  /** Seconds until it lifts, counted from when the failure arrived. */
  resets_in_seconds: z.number().optional().catch(undefined),
})

const upstreamSchema = limitFieldsSchema.extend({
  error: limitFieldsSchema.optional().catch(undefined),
  headers: z.record(z.string(), z.string()).optional().catch(undefined),
  body: z.looseObject({ error: limitFieldsSchema.optional().catch(undefined) })
    .optional()
    .catch(undefined),
})

/**
 * The moment the vendor says the request can succeed again, as an ISO time, or
 * null when it names none. A relative wait (`resets_in_seconds`, a
 * `Retry-After` in seconds) counts from `receivedAt`, the time the failure was
 * recorded.
 */
export function retryAtFromUpstream(
  upstream: string | undefined,
  receivedAt: string
): string | null {
  if (!upstream)
    return null
  let payload: unknown
  try {
    payload = JSON.parse(upstream)
  } catch {
    return null
  }
  const parsed = upstreamSchema.safeParse(payload)
  if (!parsed.success)
    return null
  const received = Date.parse(receivedAt)
  const limits = [parsed.data.error, parsed.data.body?.error, parsed.data]
  for (const limit of limits) {
    if (limit?.resets_at !== undefined)
      return new Date(limit.resets_at * 1000).toISOString()
    if (limit?.resets_in_seconds !== undefined && Number.isFinite(received))
      return new Date(received + limit.resets_in_seconds * 1000).toISOString()
  }
  const waitMs = retryAfterMsFromHeader(parsed.data.headers?.['retry-after'] ?? null)
  if (waitMs !== undefined && Number.isFinite(received))
    return new Date(received + waitMs).toISOString()
  return null
}
