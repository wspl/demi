import { createCipheriv, createDecipheriv, randomBytes } from 'node:crypto'

/**
 * Credential encryption at rest: AES-256-GCM over portable JSON, packed as
 * `v1:<iv>:<tag>:<ciphertext>` (base64 fields). Decryption failures throw —
 * a corrupt credential row is a loud error, never silently normalized.
 */
export function encryptJson(secret: Uint8Array, value: unknown): string {
  const iv = randomBytes(12)
  const cipher = createCipheriv('aes-256-gcm', secret, iv)
  const ciphertext = Buffer.concat([cipher.update(JSON.stringify(value), 'utf8'), cipher.final()])
  return `v1:${iv.toString('base64')}:${cipher.getAuthTag().toString('base64')}:${ciphertext.toString('base64')}`
}

export function decryptJson<T>(secret: Uint8Array, packed: string): T {
  const [version, iv, tag, ciphertext] = packed.split(':')
  if (version !== 'v1' || !iv || !tag || !ciphertext) {
    throw new Error('Corrupt encrypted credential: unrecognized format')
  }
  const decipher = createDecipheriv('aes-256-gcm', secret, Buffer.from(iv, 'base64'))
  decipher.setAuthTag(Buffer.from(tag, 'base64'))
  const plain = Buffer.concat([decipher.update(Buffer.from(ciphertext, 'base64')), decipher.final()])
  return JSON.parse(plain.toString('utf8')) as T
}
