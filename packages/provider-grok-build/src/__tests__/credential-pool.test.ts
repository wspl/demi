import { expect, test } from 'bun:test'
import { createProviderQuota } from '@demicodes/provider'
import { MemoryCredentialPool } from '@demicodes/provider/credentials-pool'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { FileGrokAuthStore, GrokAuthError } from '../auth'
import {
  createGrokBuildCredentials,
  PoolAwareGrokAuthStore,
} from '../credentials'

const NOW = new Date('2026-01-01T00:00:00.000Z')
const ENTRY_KEY = 'https://auth.x.ai::client'

function entry(email: string, key: string, expiresAt = '2099-01-01T00:00:00.000Z') {
  return {
    key,
    auth_mode: 'oidc',
    refresh_token: `refresh-${key}`,
    email,
    oidc_issuer: 'https://auth.x.ai',
    oidc_client_id: 'client',
    expires_at: expiresAt,
  }
}

function authText(email: string, key: string, expiresAt?: string): string {
  return `${JSON.stringify({ [ENTRY_KEY]: entry(email, key, expiresAt) })}\n`
}

function meta(id: string) {
  return {
    id,
    label: id,
    updatedAt: NOW.toISOString(),
    identityKey: ENTRY_KEY,
  }
}

async function poolWith(
  vendorDefault: boolean,
  accounts: Array<{ id: string; text: string }>
): Promise<MemoryCredentialPool> {
  const pool = new MemoryCredentialPool(vendorDefault)
  for (const account of accounts) {
    await pool.writeEntry(meta(account.id), account.text)
  }
  return pool
}

test('a pinned credentialId resolves that account, not the active one', async () => {
  const pool = await poolWith(false, [
    { id: 'a', text: authText('a@x.ai', 'token-a') },
    { id: 'b', text: authText('b@x.ai', 'token-b') },
  ])
  await pool.setActiveId('a')

  const pinned = new PoolAwareGrokAuthStore(pool, { credentialId: 'b' })
  const auth = await pinned.resolveAuth()
  expect(auth.accessToken).toBe('token-b')
  expect(auth.email).toBe('b@x.ai')
  expect(auth.entryKey).toBe(ENTRY_KEY)
  expect(auth.authFile).toBe('account b')
  expect((await new PoolAwareGrokAuthStore(pool).resolveAuth()).accessToken)
    .toBe('token-a')
  expect(await pool.getActiveId()).toBe('a')
})

test('a refresh whose replace loses uses the stored winner tokens', async () => {
  const pool = await poolWith(false, [
    { id: 'a', text: authText('a@x.ai', 'stale', '2026-01-01T00:01:00.000Z') },
  ])
  let refreshes = 0
  const store = new FileGrokAuthStore({
    document: pool.document('a'),
    entryKey: ENTRY_KEY,
    now: () => NOW,
    refresh: async () => {
      refreshes += 1
      // Another writer stores first, moving the document version on.
      await pool.writeEntry(meta('a'), authText('a@x.ai', 'winner'))
      return {
        access_token: 'loser',
        refresh_token: 'refresh-loser',
        expires_in: 3600
      }
    },
  })

  const auth = await store.resolveAuth()
  expect(refreshes).toBe(1)
  expect(auth.accessToken).toBe('winner')
  expect(auth.refreshToken).toBe('refresh-winner')
  expect((await pool.document('a').read())!.text)
    .toBe(authText('a@x.ai', 'winner'))
})

test('a refresh kept by the document stores the new tokens', async () => {
  const pool = await poolWith(false, [
    { id: 'a', text: authText('a@x.ai', 'stale', '2026-01-01T00:01:00.000Z') },
  ])
  const store = new FileGrokAuthStore({
    document: pool.document('a'),
    now: () => NOW,
    refresh: async (input) => {
      expect(input.refreshToken).toBe('refresh-stale')
      return { access_token: 'fresh', expires_in: 3600 }
    },
  })

  const auth = await store.resolveAuth()
  expect(auth.accessToken).toBe('fresh')
  expect(auth.expiresAt?.toISOString()).toBe('2026-01-01T01:00:00.000Z')
  const stored = JSON.parse((await pool.document('a').read())!.text)
  expect(stored[ENTRY_KEY].key).toBe('fresh')
  expect(stored[ENTRY_KEY].refresh_token).toBe('refresh-stale')
})

test('a refresh refused while the document changed uses the stored tokens', async () => {
  const pool = await poolWith(false, [
    { id: 'a', text: authText('a@x.ai', 'stale', '2026-01-01T00:01:00.000Z') },
  ])
  const refused = new GrokAuthError('auth_refresh_failed', 'refused')
  const store = new FileGrokAuthStore({
    document: pool.document('a'),
    now: () => NOW,
    refresh: async () => {
      // Another writer stores first, moving the document version on.
      await pool.writeEntry(meta('a'), authText('a@x.ai', 'winner'))
      throw refused
    },
  })
  expect((await store.resolveAuth()).accessToken).toBe('winner')

  const unchanged = new FileGrokAuthStore({
    document: pool.document('a'),
    now: () => NOW,
    refresh: async () => {
      throw refused
    },
  })
  await expect(unchanged.resolveAuth({ forceRefresh: true }))
    .rejects.toBe(refused)
})

test('a refresh that waited behind another uses the tokens it finds', async () => {
  const pool = await poolWith(false, [
    { id: 'a', text: authText('a@x.ai', 'stale', '2026-01-01T00:01:00.000Z') },
  ])
  let refreshes = 0
  const options = {
    document: pool.document('a'),
    now: () => NOW,
    refresh: async () => {
      refreshes += 1
      return { access_token: 'fresh', expires_in: 3600 }
    },
  }
  const [first, second] = await Promise.all([
    new FileGrokAuthStore(options).resolveAuth(),
    new FileGrokAuthStore(options).resolveAuth(),
  ])
  expect(refreshes).toBe(1)
  expect(first.accessToken).toBe('fresh')
  expect(second.accessToken).toBe('fresh')
})

test('an empty pool without a vendor default never reads the vendor home', async () => {
  const grokHome = await mkdtemp(join(tmpdir(), 'demi-grok-home-'))
  try {
    await writeFile(
      join(grokHome, 'auth.json'),
      authText('vendor@x.ai', 'vendor-token')
    )
    const pool = new MemoryCredentialPool(false)
    const authStore = new PoolAwareGrokAuthStore(pool, { grokHome })
    const credentials = createGrokBuildCredentials(
      pool,
      authStore,
      { grokHome }
    )

    expect((await authStore.status()).status).toBe('unauthenticated')
    await expect(authStore.resolveAuth()).rejects.toMatchObject({
      code: 'auth_missing'
    })
    expect(credentials.capability()).toMatchObject({ canImportDefault: false })
    await expect(credentials.importDefault!()).rejects.toMatchObject({
      code: 'auth_unsupported'
    })
    expect(await pool.list()).toEqual([])

    const lent = new PoolAwareGrokAuthStore(
      new MemoryCredentialPool(true),
      { grokHome }
    )
    expect((await lent.resolveAuth()).accessToken).toBe('vendor-token')
  } finally {
    await rm(grokHome, { recursive: true, force: true })
  }
})

test('pinned credentials keep the quota when the active account changes', async () => {
  const pool = await poolWith(false, [
    { id: 'a', text: authText('a@x.ai', 'token-a') },
    { id: 'b', text: authText('b@x.ai', 'token-b') },
  ])
  const quotaFor = async () => {
    const quota = createProviderQuota({
      providerId: 'grok-build',
      canProbe: true,
      probe: async () => ({ accountLabel: 'a@x.ai', windows: [] }),
      observe: () => null,
    })
    await quota.probe()
    return quota
  }

  const pinnedQuota = await quotaFor()
  const pinned = createGrokBuildCredentials(
    pool,
    new PoolAwareGrokAuthStore(pool, { credentialId: 'a' }),
    { quota: pinnedQuota, pinned: true }
  )
  const active = await pinned.setActive('b')
  expect(active.credentialId).toBe('b')
  expect(pinnedQuota.latest()).not.toBeNull()
  await pinned.remove!('b')
  expect(pinnedQuota.latest()).not.toBeNull()

  const followingQuota = await quotaFor()
  const following = createGrokBuildCredentials(
    pool,
    new PoolAwareGrokAuthStore(pool),
    { quota: followingQuota }
  )
  await following.setActive('a')
  expect(followingQuota.latest()).toBeNull()
})
