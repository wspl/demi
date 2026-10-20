import { coded, shownError, type ShownError } from '@demicodes/web-ui/infra/errors'

/** Lowercase Crockford base-32, the alphabet of the backend's request IDs. */
const ALPHABET = '0123456789abcdefghjkmnpqrstvwxyz'

/**
 * A request ID as the backend gives one, `req_` and 16 lowercase base-32
 * characters, for a specimen that stands for a failed request
 * (`crates/backend-http/src/request_id.rs`). The same words give the same
 * ID, so a specimen looks the same each time it is shown.
 */
export function simulatedRequestId(seed: string): string {
  let state = 2166136261
  for (const char of seed) {
    state = Math.imul(state ^ char.charCodeAt(0), 16777619) >>> 0
  }
  let id = 'req_'
  for (let index = 0; index < 16; index += 1) {
    state = Math.imul(state ^ index, 16777619) >>> 0
    id += ALPHABET[state % ALPHABET.length]
  }
  return id
}

/** A failure the backend answered, as the product shows it: its words and its request. */
export function requestFailure(message: string): ShownError {
  return { message, requestId: simulatedRequestId(message) }
}

/**
 * A refusal of the backend's as the page shows it: the catalogue's words for
 * `code`, filled with `params` and worded again when the language changes,
 * and the request it answered.
 */
export function refusalFailure(code: string, params: Readonly<Record<string, string | number>> = {}): ShownError {
  return shownError(refusal(code, params))
}

/** A refusal of the backend's as an error, for a specimen that reports it as the product does (`reportError`). */
export function refusal(code: string, params: Readonly<Record<string, string | number>> = {}): Error {
  return Object.assign(coded(new Error(code), code, params), { requestId: simulatedRequestId(code) })
}

/**
 * A refusal the user corrects in the form they filled, such as a wrong
 * password: it names no request (`crates/backend-http/src/request_id.rs`).
 */
export function correction(message: string): ShownError {
  return { message, requestId: null }
}
