import { base64ToBytes, decodeUtf8 } from './bytes'

/**
 * The claims of a JWT, read without verifying anything: the middle segment is
 * base64url-decoded and parsed as JSON. Returns `null` when the token is not
 * three segments or its payload is not JSON.
 *
 * The signature is **not** checked, so the claims are untrusted input like any
 * other network value: the caller validates the ones it reads against a
 * schema. `utils` is zero-dependency and cannot do that here.
 */
export function decodeJwtPayload(token: string): unknown {
  const segments = token.split('.')
  if (segments.length !== 3)
    return null
  const payload = segments[1]
  if (!payload)
    return null
  const base64 = payload.replace(/-/g, '+').replace(/_/g, '/')
  const padded = base64.padEnd(Math.ceil(base64.length / 4) * 4, '=')
  try {
    return JSON.parse(decodeUtf8(base64ToBytes(padded)))
  } catch {
    // Not base64url, not UTF-8, or not JSON: no claims to read.
    return null
  }
}
