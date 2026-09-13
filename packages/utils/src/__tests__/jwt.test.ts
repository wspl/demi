import { describe, expect, it } from 'bun:test'
import { bytesToBase64, encodeUtf8 } from '../bytes'
import { decodeJwtPayload } from '../jwt'

/** A JWT whose payload is `claims`, signature not included in the encoding. */
function jwtWith(claims: unknown): string {
  const base64url = bytesToBase64(encodeUtf8(JSON.stringify(claims)))
    .replace(/\+/g, '-')
    .replace(/\//g, '_')
    .replace(/=+$/, '')
  return `header.${base64url}.signature`
}

describe('decodeJwtPayload', () => {
  it('reads the claims of a base64url payload', () => {
    expect(decodeJwtPayload(jwtWith({ sub: 'user-1', exp: 1_700_000_000 })))
      .toEqual({ sub: 'user-1', exp: 1_700_000_000 })
  })

  it('reads a payload whose base64url needs padding and non-ASCII text', () => {
    expect(decodeJwtPayload(jwtWith({ email: 'zoé@example.com' })))
      .toEqual({ email: 'zoé@example.com' })
  })

  it('returns null when the token is not three segments', () => {
    expect(decodeJwtPayload('header.payload')).toBeNull()
    expect(decodeJwtPayload('')).toBeNull()
    expect(decodeJwtPayload('a..c')).toBeNull()
  })

  it('returns null when the payload is not base64url JSON', () => {
    expect(decodeJwtPayload('header.!!!!.signature')).toBeNull()
    expect(decodeJwtPayload(`header.${bytesToBase64(encodeUtf8('nope'))}.sig`))
      .toBeNull()
  })

  it('does not claim the payload is an object', () => {
    // The caller validates: a signature-less decode can yield any JSON value.
    expect(decodeJwtPayload(jwtWith(42))).toBe(42)
  })
})
