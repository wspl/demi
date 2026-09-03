import { chmodSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

/**
 * The instance secret encrypting `providers.config` at rest: generated into
 * the data directory on first start, a shared config value across instances
 * at N>1. Cheap protection against the database file leaking alone — no KMS
 * or per-user key machinery by design.
 */
export function loadOrCreateInstanceSecret(dataDir: string): Uint8Array {
  const path = join(dataDir, 'instance-secret')
  if (existsSync(path)) {
    const hex = readFileSync(path, 'utf8').trim()
    if (!/^[0-9a-f]{64}$/.test(hex)) {
      throw new Error(`Corrupt instance secret at ${path}: expected 64 hex characters`)
    }
    return Uint8Array.from(Buffer.from(hex, 'hex'))
  }
  const secret = crypto.getRandomValues(new Uint8Array(32))
  mkdirSync(dataDir, { recursive: true })
  writeFileSync(path, `${Buffer.from(secret).toString('hex')}\n`, { mode: 0o600 })
  chmodSync(path, 0o600)
  return secret
}
