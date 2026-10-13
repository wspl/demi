import type { ShownError } from '@demicodes/web-ui/infra/errors'

/** Lowercase Crockford base-32, the alphabet of the backend's request IDs. */
const ALPHABET = '0123456789abcdefghjkmnpqrstvwxyz'

/**
 * A request ID as the backend gives one, `req_` and 16 lowercase base-32
 * characters, for a specimen that stands for a failed request
 * (`backend.md` § Request IDs and the server log). The same words give the
 * same ID, so a specimen looks the same each time it is shown.
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
 * A refusal the user corrects in the form they filled, such as a wrong
 * password: it names no request (`backend.md` § Request IDs and the server
 * log).
 */
export function correction(message: string): ShownError {
  return { message, requestId: null }
}
