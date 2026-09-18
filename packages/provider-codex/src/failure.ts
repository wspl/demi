import type { ProviderErrorDiagnostics, ProviderFailureFacts } from '@demicodes/core'
import { readHttpFailure, readHttpFailureRecord } from '@demicodes/provider'
import { parseJsonOrString } from '@demicodes/utils'
import { z } from 'zod'

/**
 * A Codex usage limit says when it lifts: `resets_at` in epoch seconds and
 * `resets_in_seconds` from when it was sent. Any other value reads as absent.
 */
const codexLimitSchema = z.looseObject({
  resets_at: z.number().optional().catch(undefined),
  resets_in_seconds: z.number().optional().catch(undefined),
})

/**
 * Where the limit sits in a Codex failure: the error of a stream `error`
 * event, of a WebSocket envelope's `event`, of a `response.failed` event's
 * response, or of an HTTP failure's body.
 */
const codexFailureSchema = z.looseObject({
  error: codexLimitSchema.optional().catch(undefined),
  event: z.looseObject({ error: codexLimitSchema.optional().catch(undefined) })
    .optional()
    .catch(undefined),
  response: z.looseObject({ error: codexLimitSchema.optional().catch(undefined) })
    .optional()
    .catch(undefined),
})

/**
 * Reads a failure record the Codex provider produced
 * (`docs/provider-errors-and-retries.md` § Reading a failure): the usage
 * limit's reset, and otherwise the standard reading of an HTTP failure.
 */
export function readCodexFailure(
  diagnostics: ProviderErrorDiagnostics,
  receivedAt: string
): ProviderFailureFacts {
  const limit = codexLimit(diagnostics)
  if (limit?.resets_at !== undefined)
    return { retryAt: new Date(limit.resets_at * 1000).toISOString() }
  if (limit?.resets_in_seconds !== undefined) {
    const lifts = Date.parse(receivedAt) + limit.resets_in_seconds * 1000
    return { retryAt: new Date(lifts).toISOString() }
  }
  return readHttpFailure(diagnostics, receivedAt)
}

/** The usage limit a failure record carries, when it carries one. */
function codexLimit(
  diagnostics: ProviderErrorDiagnostics
): z.infer<typeof codexLimitSchema> | undefined {
  const text = diagnostics.source === 'http'
    ? readHttpFailureRecord(diagnostics)?.body
    : diagnostics.upstream
  if (!text)
    return undefined
  const failure = codexFailureSchema.safeParse(parseJsonOrString(text))
  if (!failure.success)
    return undefined
  return failure.data.error
    ?? failure.data.event?.error
    ?? failure.data.response?.error
}
