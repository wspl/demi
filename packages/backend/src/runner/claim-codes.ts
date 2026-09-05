/**
 * Pairing-code and device-token primitives (design: demi-next.md § Runner
 * program). Claim codes are 128-bit random values in Crockford base32 —
 * guessing is infeasible by entropy alone; single-use/expiry/rate-limiting
 * live in the registry. Device tokens are 256-bit random hex, stored only as
 * a SHA-256 hash.
 */

const CROCKFORD_ALPHABET = '0123456789ABCDEFGHJKMNPQRSTVWXYZ'

/** 128 random bits as 26 Crockford base32 characters, dash-grouped for copy-paste. */
export function generateClaimCode(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16))
  let bits = 0
  let acc = 0
  let out = ''
  for (const byte of bytes) {
    acc = (acc << 8) | byte
    bits += 8
    while (bits >= 5) {
      bits -= 5
      out += CROCKFORD_ALPHABET[(acc >> bits) & 31]
    }
  }
  if (bits > 0) out += CROCKFORD_ALPHABET[(acc << (5 - bits)) & 31]
  return out.replace(/(.{4})(?=.)/g, '$1-')
}

/** Uppercases, strips separators, and maps the Crockford confusables (O→0, I/L→1). */
export function normalizeClaimCode(code: string): string {
  return code
    .toUpperCase()
    .replace(/[\s-]/g, '')
    .replace(/O/g, '0')
    .replace(/[IL]/g, '1')
}

export function generateDeviceToken(): string {
  return Buffer.from(crypto.getRandomValues(new Uint8Array(32))).toString('hex')
}

export function hashDeviceToken(token: string): string {
  return new Bun.CryptoHasher('sha256').update(token).digest('hex')
}
