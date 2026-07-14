// Shared building blocks for HTTP-based provider adapters: secret redaction and
// coarse error-code classification. Provider implementations import these instead
// of re-deriving the same status/keyword tables.
import { shortHash } from '@demicodes/utils'
import type { ProviderAuthState, ProviderEvent } from './types'

type SecretResolver = () => string | Promise<string> | null | undefined
type HeadersResolver = () => Record<string, string> | Promise<Record<string, string>>

/**
 * Clamps a session identifier to the 64-character limit the OpenAI Responses
 * API enforces on `prompt_cache_key` (and on the headers it derives it from).
 */
export function clampPromptCacheKey(value: string): string {
  return value.length <= 64 ? value : `session_${shortHash(value)}`
}

/** Replaces every occurrence of `secret` in `value` with a redaction marker. */
export function redactSecretText(value: string, secret: string | null | undefined): string {
  return secret ? value.split(secret).join('[redacted]') : value
}

/** Masks bearer tokens and named credential fields in free-form auth error text. */
export function redactCredentialText(text: string, extraFieldPatterns: readonly string[] = []): string {
  const fields = ['access_token', 'refresh_token', 'id_token', ...extraFieldPatterns]
  return text
    .replace(/Bearer\s+[A-Za-z0-9._~+/=-]+/g, 'Bearer [REDACTED]')
    .replace(new RegExp(`(${fields.join('|')})["'=:\\s]+[A-Za-z0-9._~+/=-]+`, 'gi'), '$1=[REDACTED]')
}

/** Maps an HTTP status (and response text) to a coarse provider error code. */
export function httpErrorCode(status: number, message: string): string | null {
  if (status === 401 || status === 403) return 'auth_expired'
  if (status === 408 || status === 409 || status === 425 || status === 429 || status >= 500) return 'rate_limit'
  if (status === 400 && /context|too long|token/i.test(message)) return 'context_length_exceeded'
  return null
}

/** Classifies a provider error code/message into a coarse category, falling back to `code`. */
export function normalizeErrorCode(code: string | null, message: string): string | null {
  const value = `${code ?? ''} ${message}`.toLowerCase()
  if (/context|too long|max.*token/.test(value)) return 'context_length_exceeded'
  if (/rate|quota|usage|billing|balance|limit/.test(value)) return 'rate_limit'
  if (/auth|invalid.*(key|token)|expired/.test(value)) return 'auth_expired'
  // Transport-level transient failures (header/connect/idle timeouts, dropped
  // sockets, DNS/connect errors) classify as overloaded so the turn retry
  // policy treats them like a 503 instead of surfacing a terminal error.
  if (/overload|unavailable|timed?\s?out|fetch failed|network|socket|econn/.test(value)) return 'overloaded'
  return code
}

/** Builds a provider `error` event from an unknown thrown value, redacting `secret`. */
export function providerErrorFromUnknown(error: unknown, secret: string | null | undefined): ProviderEvent {
  const message = error instanceof Error ? error.message : String(error)
  return { type: 'error', message: redactSecretText(message, secret), code: normalizeErrorCode(null, message) }
}

/** Resolves auth state from an API key or a matching custom auth header. */
export async function authStatusFromKey(
  resolveKey: SecretResolver,
  resolveHeaders: HeadersResolver | undefined,
  authHeader: string,
  providerLabel: string,
): Promise<ProviderAuthState> {
  const [key, headers] = await Promise.all([resolveKey(), resolveHeaders?.()])
  if (key || (headers && Object.keys(headers).some((name) => name.toLowerCase() === authHeader))) {
    return { status: 'authenticated' }
  }
  return { status: 'unauthenticated', message: `${providerLabel} API key is missing` }
}

/** Parses a Retry-After header (delta-seconds or HTTP-date) into milliseconds. */
export function retryAfterMsFromHeader(value: string | null): number | undefined {
  if (!value) return undefined
  const seconds = Number(value)
  if (Number.isFinite(seconds) && seconds >= 0) return Math.floor(seconds * 1000)
  const dateMs = Date.parse(value)
  if (Number.isFinite(dateMs)) return Math.max(0, dateMs - Date.now())
  return undefined
}

/** Reads a header as a finite number, or null when absent/blank/non-numeric. */
export function numberHeader(headers: Headers, name: string): number | null {
  const raw = headers.get(name)
  if (raw == null || raw === '') return null
  const n = Number(raw)
  return Number.isFinite(n) ? n : null
}

/** Builds a redacted provider `error` event from a failed HTTP response. */
export async function httpRequestFailedEvent(
  response: Response,
  secret: string | null | undefined,
  providerLabel: string,
): Promise<ProviderEvent> {
  const text = await response.text().catch(() => '')
  const message = redactSecretText(
    `${providerLabel} API request failed with HTTP ${response.status}${text ? `: ${text}` : ''}`,
    secret,
  )
  const retryAfterMs = retryAfterMsFromHeader(response.headers.get('retry-after'))
  const event: ProviderEvent = { type: 'error', message, code: httpErrorCode(response.status, message) }
  if (retryAfterMs !== undefined) event.retryAfterMs = retryAfterMs
  return event
}
