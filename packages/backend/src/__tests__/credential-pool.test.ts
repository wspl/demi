import { FileCredentialPool } from '@demicodes/provider/credentials-pool'
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { LocalControlService } from '../storage/control'
import { openSqliteDatabase } from '../storage/database'
import { CONTROL_MIGRATIONS, migrate } from '../storage/migrations'
import { AccountQuotas } from '../vault/credential-pool'
import { importCredentialPools } from '../vault/import-pools'
import { ProviderVault } from '../vault/providers'

async function fixture() {
  const db = openSqliteDatabase(':memory:')
  migrate(db, CONTROL_MIGRATIONS)
  const control = new LocalControlService(db)
  const owner = (await control.createMaster(
    { email: 'owner@example.test', passwordHash: '!' }
  ))!
  const secret = crypto.getRandomValues(new Uint8Array(32))
  const vault = new ProviderVault(control, secret, 'isolated')
  const entry = await vault.create({
    ownerUserId: owner.id,
    label: 'Codex',
    config: { kind: 'subscription', providerType: 'codex' }
  })
  return { db, control, secret, vault, entry }
}

const meta = (id: string, identityKey = id) => ({
  id,
  label: `${id}@example.test`,
  identityKey,
  updatedAt: new Date().toISOString()
})

test('an account is an encrypted record whose refreshed secret is stored only over the version it was read at', async () => {
  const { db, vault, entry } = await fixture()
  const pool = vault.credentialPool(entry.id)
  expect(await pool.ensureActivePointer()).toBeNull()
  await pool.writeEntry(meta('a'), '{"refresh":"one"}')
  expect(await pool.ensureActivePointer()).toBe('a')
  expect(db.get<{ secret: string }>('SELECT secret FROM provider_credentials')!.secret)
    .not.toContain('one')

  const first = await pool.document('a').read()
  const second = await pool.document('a').read()
  expect(first?.text).toBe('{"refresh":"one"}')
  // Two refreshers read the same revision; the second write finds a newer one.
  expect(await pool.document('a').replace('{"refresh":"two"}', first!.version)).toBe(true)
  expect(await pool.document('a').replace('{"refresh":"lost"}', second!.version)).toBe(false)
  expect((await pool.document('a').read())?.text).toBe('{"refresh":"two"}')

  // Another entry's pool cannot name this account.
  const other = vault.credentialPool('another-entry')
  expect(await other.document('a').read()).toBeNull()
  expect(await other.list()).toEqual([])

  await pool.writeEntry(meta('b'), '{}')
  await expect(pool.setActiveId('missing')).rejects.toThrow('not found')
  await pool.setActiveId('b')
  await pool.remove('b')
  expect(await pool.getActiveId()).toBeNull()
  await vault.delete(entry.id)
  expect(db.all('SELECT id FROM provider_credentials')).toEqual([])
  db.close()
})

test('usage is kept per account and shared by every provider built for it', async () => {
  const { db, control, vault, entry } = await fixture()
  const pool = vault.credentialPool(entry.id)
  await pool.writeEntry(meta('a'), '{}')
  await pool.writeEntry(meta('b'), '{}')
  const quotas = new AccountQuotas(control)
  const [a, b] = await vault.accounts(entry.id)
  const snapshot = {
    providerId: entry.id,
    observedAt: new Date().toISOString(),
    source: 'probe' as const,
    windows: [{ id: 'weekly', usedPercent: 40, resetsAt: null }],
    raw: { vendor: 'payload' }
  }
  quotas.keeper(entry.id, a!).save(snapshot)
  expect(quotas.keeper(entry.id, a!).read()).toMatchObject({ windows: [{ usedPercent: 40 }] })
  expect(quotas.keeper(entry.id, b!).read()).toBeNull()
  await Bun.sleep(5)
  // A restarted backend starts from the record, which never holds the raw payload.
  const stored = (await vault.account(entry.id, 'a'))!
  expect(stored.quota).not.toContain('payload')
  expect(new AccountQuotas(control).latest(entry.id, stored))
    .toMatchObject({ windows: [{ id: 'weekly', usedPercent: 40 }] })
  db.close()
})

test('a pool directory of an older backend becomes account records and is removed', async () => {
  const { db, control, secret, vault, entry } = await fixture()
  const vaultRoot = await mkdtemp(join(tmpdir(), 'demi-vault-'))
  const files = new FileCredentialPool({
    stateDir: join(vaultRoot, entry.id),
    providerKey: 'codex',
    secretFileName: 'auth.json'
  })
  await files.writeEntry(meta('cred-a'), '{"token":"a"}')
  await files.writeEntry(meta('cred-b'), '{"token":"b"}')
  await files.setActiveId('cred-b')
  await writeFile(join(vaultRoot, entry.id, 'quota.json'), JSON.stringify({
    providerId: entry.id,
    observedAt: '2026-09-20T00:00:00.000Z',
    source: 'probe',
    plan: { id: 'pro', label: 'Pro' },
    windows: []
  }))
  await mkdir(join(vaultRoot, 'pending-abandoned'), { recursive: true })

  await importCredentialPools(control, secret, vaultRoot)
  const pool = vault.credentialPool(entry.id)
  expect((await pool.list()).map(account => account.id)).toEqual(['cred-a', 'cred-b'])
  expect(await pool.getActiveId()).toBe('cred-b')
  expect((await pool.document('cred-a').read())?.text).toBe('{"token":"a"}')
  expect((await vault.account(entry.id, 'cred-b'))?.quota).toContain('Pro')
  expect((await vault.account(entry.id, 'cred-a'))?.quota).toBeNull()
  expect(existsSync(join(vaultRoot, entry.id))).toBe(false)
  expect(existsSync(join(vaultRoot, 'pending-abandoned'))).toBe(false)
  // Starting again finds nothing to do.
  await importCredentialPools(control, secret, vaultRoot)
  db.close()
})
