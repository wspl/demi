import { mkdtempSync, readFileSync, statSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { LocalControlService } from '../storage/control'
import { openSqliteDatabase } from '../storage/database'
import { CONTROL_MIGRATIONS, migrate } from '../storage/migrations'
import { ConnectionVault } from '../vault/connections'
import { decryptJson, encryptJson } from '../vault/crypto'
import { loadOrCreateInstanceSecret } from '../vault/secret'

test('instance secret: generated once with 0600, stable across loads, corrupt file is loud', () => {
  const dataDir = mkdtempSync(join(tmpdir(), 'demi-vault-secret-'))
  const first = loadOrCreateInstanceSecret(dataDir)
  expect(first).toHaveLength(32)
  expect(statSync(join(dataDir, 'instance-secret')).mode & 0o777).toBe(0o600)
  const second = loadOrCreateInstanceSecret(dataDir)
  expect(Buffer.from(second).toString('hex')).toBe(Buffer.from(first).toString('hex'))

  const corruptDir = mkdtempSync(join(tmpdir(), 'demi-vault-corrupt-'))
  loadOrCreateInstanceSecret(corruptDir)
  const path = join(corruptDir, 'instance-secret')
  require('node:fs').writeFileSync(path, 'nonsense')
  expect(() => loadOrCreateInstanceSecret(corruptDir)).toThrow('Corrupt instance secret')
})

test('crypto: round trip, unique ciphertexts, wrong key and tampering fail loudly', () => {
  const secret = crypto.getRandomValues(new Uint8Array(32))
  const value = { kind: 'api_key', provider: 'openai', apiKey: 'sk-test-123' }
  const packed = encryptJson(secret, value)
  expect(packed.startsWith('v1:')).toBe(true)
  expect(packed).not.toContain('sk-test-123')
  expect(decryptJson<typeof value>(secret, packed)).toEqual(value)
  // Fresh IV every call.
  expect(encryptJson(secret, value)).not.toBe(packed)

  const otherKey = crypto.getRandomValues(new Uint8Array(32))
  expect(() => decryptJson(otherKey, packed)).toThrow()
  const [v, iv, tag, ct] = packed.split(':')
  const flipped = Buffer.from(ct!, 'base64')
  flipped[0]! ^= 0xff
  expect(() => decryptJson(secret, `${v}:${iv}:${tag}:${flipped.toString('base64')}`)).toThrow()
})

test('ConnectionVault: rows carry ciphertext only; CRUD round-trips typed configs', async () => {
  const db = openSqliteDatabase(':memory:')
  migrate(db, CONTROL_MIGRATIONS)
  const control = new LocalControlService(db)
  await control.ensureUser({ id: 'u1', username: 'local', role: 'master' })
  const vault = new ConnectionVault(control, crypto.getRandomValues(new Uint8Array(32)))

  const created = await vault.create({
    ownerUserId: 'u1',
    label: 'My OpenAI',
    config: { kind: 'api_key', provider: 'openai', apiKey: 'sk-secret', baseUrl: 'https://proxy.example/v1' },
  })
  expect(created.config.apiKey).toBe('sk-secret')

  // The database never sees the key.
  const raw = db.get<{ config: string; type: string }>('SELECT config, type FROM connections WHERE id = ?', [created.id])
  expect(raw?.config).not.toContain('sk-secret')
  expect(raw?.type).toBe('openai')

  expect((await vault.get(created.id))?.config).toEqual(created.config)
  expect((await vault.list()).map((connection) => connection.id)).toEqual([created.id])
  await vault.delete(created.id)
  expect(await vault.get(created.id)).toBeNull()
  db.close()
})
