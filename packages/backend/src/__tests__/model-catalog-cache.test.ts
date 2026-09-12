import { expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { ModelCatalogCache, MODEL_CATALOG_TTL_MS } from '../llm/model-catalog-cache'
import { LocalControlService } from '../storage/control'
import { openSqliteDatabase } from '../storage/database'
import { CONTROL_MIGRATIONS, migrate } from '../storage/migrations'
import type { CatalogSnapshot } from '../storage/model-catalog'

const snapshot = (id = 'provider', name = 'First'): CatalogSnapshot => ({
  sourceFetchedAt: '2026-09-13T00:00:00.000Z', stale: false, warnings: [],
  models: [{
    providerId: id, id: 'model', displayName: name,
    contextWindow: 1000, outputLimit: null,
    supportsTools: true, supportsAttachments: false, supportsReasoning: false,
    supportedThinkingEfforts: [], defaultThinkingEffort: null,
    sourceFetchedAt: '2026-09-13T00:00:00.000Z', stale: false,
  }],
})
async function fixture(path = ':memory:') {
  const db = openSqliteDatabase(path)
  migrate(db, CONTROL_MIGRATIONS)
  const control = new LocalControlService(db)
  if (!await control.getProvider('provider')) {
    await control.createProvider({
      id: 'provider', ownerUserId: null, providerType: 'fake', credentialKind: 'api_key',
      label: 'Test', config: 'opaque',
    })
  }
  return { db, control }
}

test('fresh memory and database restart hits avoid the upstream; TTL refreshes once in background', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-catalog-'))
  const path = join(dir, 'control.sqlite')
  let now = 1000
  let calls = 0
  const load = async () => {
    calls++
    return snapshot()
  }
  let { db, control } = await fixture(path)
  let cache = new ModelCatalogCache(control, () => now)
  try {
    await Promise.all(Array.from({ length: 8 }, () => cache.get('provider', 'account-a', load)))
    expect(calls).toBe(1)
    await cache.get('provider', 'account-a', load)
    expect(calls).toBe(1)
    await cache.close()
    db.close()
    const restarted = await fixture(path)
    db = restarted.db
    control = restarted.control
    cache = new ModelCatalogCache(control, () => now)
    expect((await cache.get('provider', 'account-a', load)).models[0]?.displayName).toBe('First')
    expect(calls).toBe(1)
    now += MODEL_CATALOG_TTL_MS
    const next = Promise.withResolvers<CatalogSnapshot>()
    const refresh = () => {
      calls++
      return next.promise
    }
    const stale = await Promise.all(Array.from({ length: 8 }, () => cache.get('provider', 'account-a', refresh)))
    expect(calls).toBe(2)
    expect(stale.every(item => item.stale && item.models[0]?.stale)).toBe(true)
    const forced = cache.get('provider', 'account-a', refresh, true)
    next.resolve(snapshot('provider', 'Updated'))
    expect((await forced).models[0]?.displayName).toBe('Updated')
    expect(calls).toBe(2)
    expect((await control.getModelCatalog('provider'))?.checkedAt).toBe(now)
  } finally {
    await cache.close()
    db.close()
    await rm(dir, { recursive: true, force: true })
  }
})

test('failed refresh retains persisted data, cools down retries, and explicit refresh bypasses the delay', async () => {
  const { db, control } = await fixture()
  let now = 1000
  const cache = new ModelCatalogCache(control, () => now)
  let failures = 0
  const fail = async () => {
    failures++
    throw new Error('offline')
  }
  try {
    await cache.get('provider', 'key', async () => snapshot())
    const original = await control.getModelCatalog('provider')
    now += MODEL_CATALOG_TTL_MS
    const stale = await cache.get('provider', 'key', fail, true)
    expect(stale.stale).toBe(true)
    expect(stale.warnings).toContain('offline')
    expect(await control.getModelCatalog('provider')).toEqual(original)
    await cache.get('provider', 'key', fail)
    expect(failures).toBe(1)
    now += 60_000
    await cache.get('provider', 'key', fail, true)
    expect(failures).toBe(2)
    expect((await cache.get('provider', 'key', async () => snapshot('provider', 'Recovered'), true)).stale).toBe(false)
  } finally {
    await cache.close()
    db.close()
  }
})

test('identity changes and invalidation cancel old refreshes and prevent old results from overwriting storage', async () => {
  const { db, control } = await fixture()
  const cache = new ModelCatalogCache(control)
  const pending = Promise.withResolvers<CatalogSnapshot>()
  let oldSignal: AbortSignal | undefined
  try {
    const old = cache.get('provider', 'old-account', signal => {
      oldSignal = signal
      return pending.promise
    })
    const rejected = old.catch(error => error)
    await Promise.resolve()
    await Promise.resolve()
    await cache.invalidate('provider')
    expect(oldSignal?.aborted).toBe(true)
    expect(await rejected).toBeInstanceOf(Error)
    await cache.get('provider', 'new-account', async () => snapshot('provider', 'New account'))
    pending.resolve(snapshot('provider', 'Old account'))
    await Promise.resolve()
    expect((await control.getModelCatalog('provider'))?.catalog.models[0]?.displayName).toBe('New account')
    let reads = 0
    await cache.get('provider', 'changed-config', async () => {
      reads++
      return snapshot()
    })
    expect(reads).toBe(1)
    await control.deleteProvider('provider')
    expect(await control.getModelCatalog('provider')).toBeNull()
  } finally {
    await cache.close()
    db.close()
  }
})

test('cold failures and invalid data are explicit; timeout and close release pending readers', async () => {
  const { db, control } = await fixture()
  const cache = new ModelCatalogCache(control, Date.now, 5)
  try {
    await expect(cache.get('provider', 'key', async () => snapshot('other'))).rejects.toThrow('identity')
    expect(await control.getModelCatalog('provider')).toBeNull()
    let signal: AbortSignal | undefined
    await expect(cache.get('provider', 'key', input => {
      signal = input
      return new Promise(() => {})
    }, true)).rejects.toThrow('timed out')
    expect(signal?.aborted).toBe(true)
    const pending = cache.get('provider', 'key', () => new Promise(() => {}), true)
    const rejected = pending.catch(error => error)
    await Promise.resolve()
    await cache.close()
    expect(await rejected).toBeInstanceOf(Error)
    await expect(cache.get('provider', 'key', async () => snapshot())).rejects.toThrow('closed')
  } finally {
    await cache.close()
    db.close()
  }
})
