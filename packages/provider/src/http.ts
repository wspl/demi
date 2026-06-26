// Shared building blocks for HTTP-based provider adapters: secret redaction and
// coarse error-code classification. Provider implementations import these instead
// of re-deriving the same status/keyword tables.
import type { ProviderEvent } from './types'

/** Replaces every occurrence of `secret` in `value` with a redaction marker. */
export function redactSecretText(value: string, secret: string | null | undefined): string {
  return secret ? value.split(secret).join('[redacted]') : value
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
  if (/rate|quota|billing|limit/.test(value)) return 'rate_limit'
  if (/auth|unauth|invalid.*key|expired/.test(value)) return 'auth_expired'
  if (/overload|unavailable|timeout/.test(value)) return 'overloaded'
  return code
}

/** Builds a provider `error` event from an unknown thrown value, redacting `secret`. */
export function providerErrorFromUnknown(error: unknown, secret: string | null | undefined): ProviderEvent {
  const message = error instanceof Error ? error.message : String(error)
  return { type: 'error', message: redactSecretText(message, secret), code: normalizeErrorCode(null, message) }
}
