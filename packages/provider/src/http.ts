// Shared building blocks for HTTP-based provider adapters: coarse error-code
// classification, the HTTP failure record and its standard reading, and
// credential masking for auth-file text. Provider implementations import these
// instead of re-deriving the same status/keyword tables.
import { z } from 'zod'
import type { ProviderErrorDiagnostics, ProviderFailureFacts } from '@demicodes/core'
import { parseJsonOrString, shortHash } from '@demicodes/utils'
import type { ProviderAuthState, ProviderEvent, ProviderFailureReader } from './types'

type SecretResolver = () => string | Promise<string> | null | undefined
type HeadersResolver = () => Record<string, string>
  | Promise<Record<string, string>>

/**
 * Clamps a session identifier to the 64-character limit the OpenAI Responses
 * API enforces on `prompt_cache_key` (and on the headers it derives it from).
 */
export function clampPromptCacheKey(value: string): string {
  return value.length <= 64 ? value : `session_${shortHash(value)}`
}

/**
 * Masks bearer tokens and named credential fields in free-form auth error text.
 */
export function redactCredentialText(
  text: string,
  extraFieldPatterns: readonly string[] = []
): string {
  const fields = [
    'access_token',
    'refresh_token',
    'id_token',
    ...extraFieldPatterns
  ]
  return text
    .replace(/Bearer\s+[A-Za-z0-9._~+/=-]+/g, 'Bearer [REDACTED]')
    .replace(new RegExp(
      `(${fields.join('|')})["'=:\\s]+[A-Za-z0-9._~+/=-]+`,
      'gi'
    ), '$1=[REDACTED]')
}

/** Maps an HTTP status (and response text) to a coarse provider error code. */
export function httpErrorCode(status: number, message: string): string | null {
  if (status === 401 || status === 403)
    return 'auth_expired'
  if (status === 429)
    return 'rate_limit'
  if (status === 408 || status === 409 || status === 425 || status >= 500)
    return 'overloaded'
  if (status === 400 && /context|too long|token/i.test(message))
    return 'context_length_exceeded'
  return null
}

/**
 * Classifies a provider error code/message into a coarse category, falling back
 * to `code`.
 */
export function normalizeErrorCode(
  code: string | null,
  message: string
): string | null {
  const value = `${code ?? ''} ${message}`.toLowerCase()
  if (/context|too long|max.*token/.test(value))
    return 'context_length_exceeded'
  if (/rate|quota|usage|billing|balance|limit/.test(value))
    return 'rate_limit'
  if (
    /\bauth(?:entication|orization)?\b/.test(value) ||
    /(?:invalid|expired).*(?:api|access|auth)[_\s-]*(?:key|token)/.test(value) ||
    /(?:api|access|auth)[_\s-]*(?:key|token).*(?:invalid|expired)/.test(value)
  ) {
    return 'auth_expired'
  }
  // Transport-level transient failures (header/connect/idle timeouts, dropped
  // sockets, DNS/connect errors) classify as overloaded so the turn retry
  // policy treats them like a 503 instead of surfacing a terminal error.
  if (/overload|unavailable|(?:server|internal|api)[_\s-]*error|timed?\s?out|fetch failed|network|socket|econn/.test(value)) {
    return 'overloaded'
  }
  return code
}

/**
 * Builds a provider `error` event from an unknown thrown value: no answer came
 * from the vendor, so there is no failure record.
 */
export function providerErrorFromUnknown(error: unknown): ProviderEvent {
  const message = error instanceof Error ? error.message : String(error)
  return {
    type: 'error',
    message,
    code: normalizeErrorCode(null, message)
  }
}

/** Resolves auth state from an API key or a matching custom auth header. */
export async function authStatusFromKey(
  resolveKey: SecretResolver,
  resolveHeaders: HeadersResolver | undefined,
  authHeader: string,
  providerLabel: string,
): Promise<ProviderAuthState> {
  const [key, headers] = await Promise.all([resolveKey(), resolveHeaders?.()])
  if (key
    || (headers
      && Object.keys(headers)
        .some((name) => name.toLowerCase() === authHeader))) {
    return { status: 'authenticated' }
  }
  return {
    status: 'unauthenticated',
    message: `${providerLabel} API key is missing`
  }
}

/**
 * The moment a Retry-After value says to retry, as epoch milliseconds, with a
 * delay counted from `receivedAtMs`; null for a value it cannot read.
 *
 * RFC 9110 gives the header two spellings, delta-seconds (`120`) and an
 * HTTP-date (`Wed, 21 Oct 2015 07:28:00 GMT`), and a receiver must accept
 * both. Which one arrived is decided by whether it parses as a number, so the
 * conversion is the parse; there is no shape to validate against a schema.
 */
export function retryAtFromHeader(
  value: string | null,
  receivedAtMs: number
): number | null {
  if (!value)
    return null
  const seconds = Number(value)
  if (Number.isFinite(seconds) && seconds >= 0)
    return receivedAtMs + Math.floor(seconds * 1000)
  const dateMs = Date.parse(value)
  return Number.isFinite(dateMs) ? dateMs : null
}

/**
 * Reads a header as a finite number, or null when absent/blank/non-numeric.
 *
 * Headers are text with no declared type, so the numeric ones (rate-limit
 * counters and windows) are read the same way `retryAtFromHeader` reads its
 * delta-seconds: a vendor that omits one or sends a word means "no value",
 * not a protocol error.
 */
export function numberHeader(headers: Headers, name: string): number | null {
  const raw = headers.get(name)
  if (raw == null || raw === '')
    return null
  const n = Number(raw)
  return Number.isFinite(n) ? n : null
}

/**
 * The failure record of an HTTP response (`docs/provider-errors-and-retries.md`
 * § The failure record): the status, every header as the client reports it,
 * and the body text as received.
 */
export const httpFailureRecordSchema = z.object({
  status: z.number().int(),
  headers: z.array(z.tuple([z.string(), z.string()])),
  body: z.string(),
})
export type HttpFailureRecord = z.infer<typeof httpFailureRecordSchema>

/** The record an HTTP failure keeps as its `upstream`. */
export function httpFailureRecord(
  status: number,
  headers: Headers,
  body: string
): string {
  const pairs: Array<[string, string]> = []
  headers.forEach((value, name) => {
    pairs.push([name, value])
  })
  const record: HttpFailureRecord = { status, headers: pairs, body }
  return JSON.stringify(record)
}

/** The HTTP failure record a diagnostic holds, or null when it holds none. */
export function readHttpFailureRecord(
  diagnostics: ProviderErrorDiagnostics
): HttpFailureRecord | null {
  if (diagnostics.source !== 'http' || diagnostics.upstream === undefined)
    return null
  const record = httpFailureRecordSchema.safeParse(
    parseJsonOrString(diagnostics.upstream)
  )
  return record.success ? record.data : null
}

/**
 * The standard reading of a failure record: an HTTP failure's Retry-After
 * header. A provider without fields of its own uses it as its `readFailure`;
 * one with its own fields falls back to it.
 */
export function readHttpFailure(
  diagnostics: ProviderErrorDiagnostics,
  receivedAt: string
): ProviderFailureFacts {
  const retryAfter = readHttpFailureRecord(diagnostics)?.headers
    .find(([name]) => name.toLowerCase() === 'retry-after')?.[1] ?? null
  const retryAt = retryAtFromHeader(retryAfter, Date.parse(receivedAt))
  return { retryAt: retryAt === null ? null : new Date(retryAt).toISOString() }
}

/**
 * An error event with the wait its provider reads out of its failure record,
 * for the retry policy. The same reader serves the display later, so the wait
 * and what the user is told cannot disagree.
 */
export function withRetryWait(
  event: ProviderEvent,
  readFailure: ProviderFailureReader
): ProviderEvent {
  if (event.type !== 'error' || !event.diagnostics)
    return event
  const now = Date.now()
  const { retryAt } = readFailure(event.diagnostics, new Date(now).toISOString())
  if (retryAt === null)
    return event
  return { ...event, retryAfterMs: Math.max(0, Date.parse(retryAt) - now) }
}

/**
 * Builds a provider `error` event from a failed HTTP response: the vendor's
 * text in the message, the whole response in the failure record, and the wait
 * the provider's reader finds in it.
 */
export async function httpRequestFailedEvent(
  response: Response,
  providerLabel: string,
  readFailure: ProviderFailureReader,
): Promise<ProviderEvent> {
  // A body that cannot be read leaves the status and headers to speak; the
  // failure is reported either way.
  const text = await response.text().catch(() => '')
  const message = `${providerLabel} API request failed with HTTP ${response.status}${text ? `: ${text}` : ''}`
  return withRetryWait({
    type: 'error',
    message,
    code: httpErrorCode(response.status, message),
    diagnostics: {
      source: 'http',
      httpStatus: response.status,
      upstream: httpFailureRecord(response.status, response.headers, text),
    },
  }, readFailure)
}
