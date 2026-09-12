import { base64ToBytes, bytesToBase64 } from '@demicodes/utils'
import { createCipheriv, createDecipheriv, randomBytes } from 'node:crypto'

/**
 * Credential encryption at rest: AES-256-GCM over plain JSON, packed as
 * `v1:<iv>:<tag>:<ciphertext>` (base64 fields). Decryption failures throw —
 * a corrupt credential row is a loud error, never silently normalized.
 */
export function encryptJson(secret: Uint8Array, value: unknown): string {
  const iv = randomBytes(12)
  const cipher = createCipheriv('aes-256-gcm', secret, iv)
  const ciphertext = Buffer.concat([
    cipher.update(JSON.stringify(value), 'utf8'),
    cipher.final()
  ])
  return `v1:${iv.toString('base64')}:${cipher.getAuthTag().toString('base64')}:${ciphertext.toString('base64')}`
}

export function decryptJson(secret: Uint8Array, packed: string): unknown {
  const fields = packed.split(':')
  const [version, iv, tag, ciphertext] = fields
  if (fields.length !== 4 || version !== 'v1' || !iv || !tag || !ciphertext) {
    throw new Error('Corrupt encrypted credential: unrecognized format')
  }
  const ivBytes = decodeField(iv)
  const tagBytes = decodeField(tag)
  const ciphertextBytes = decodeField(ciphertext)
  if (ivBytes.length !== 12 || tagBytes.length !== 16)
    throw new Error('Corrupt encrypted credential: invalid IV or tag length')
  const decipher = createDecipheriv('aes-256-gcm', secret, ivBytes)
  decipher.setAuthTag(tagBytes)
  const plain = Buffer.concat([
    decipher.update(ciphertextBytes),
    decipher.final()
  ])
  return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(plain))
}

function decodeField(value: string): Uint8Array {
  const bytes = base64ToBytes(value)
  if (bytesToBase64(bytes) !== value)
    throw new Error('Corrupt encrypted credential: invalid base64 field')
  return bytes
}
