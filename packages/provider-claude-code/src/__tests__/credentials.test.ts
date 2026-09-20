import { expect, test } from 'bun:test'
import { createProviderQuota } from '@demicodes/provider'
import {
  CredentialPoolError,
  fallbackCredentialPool,
  MemoryCredentialPool,
} from '@demicodes/provider/credentials-pool'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createClaudeCodeProvider } from '../provider'
import {
  createClaudeCodeCredentials,
  openClaudeCodeCredentialPool,
  PoolAwareClaudeCodeAuthStore,
} from '../credentials'
import { StaticClaudeCodeAuthStore } from '../auth'
import { buildClaudeEnv } from '../cli'
import { injectableCliToken } from '../oauth'
import type { ClaudeCodeOAuthSecret } from '../secret'
import { claudeCodeVendorPool } from '../vendor'

const NO_KEYCHAIN = async () => null
const KEYCHAIN_ITEM = async () => JSON.stringify({
  claudeAiOauth: {
    accessToken: 'keychain-token',
    subscriptionType: 'max',
    rateLimitTier: 'tier-20x',
  },
})

test('claude credentials add/setActive and pool-aware resolve', async () => {
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-claude-cred-'))
  try {
    const pool = openClaudeCodeCredentialPool({ stateDir })
    const authStore = new PoolAwareClaudeCodeAuthStore(pool)
    const quota = createProviderQuota({
      providerId: 'claude-code',
      canProbe: true,
      probe: async () => ({ accountLabel: 'old-account', windows: [] }),
      observe: () => ({ accountLabel: 'old-account', windows: [] }),
    })
    const credentials = createClaudeCodeCredentials(pool, authStore, { quota })

    const a = await credentials.add!({
      accessToken: 'token-a',
      subscriptionType: 'pro'
    })
    const b = await credentials.add!({
      accessToken: 'token-b',
      subscriptionType: 'max'
    })
    expect((await credentials.list()).length).toBe(2)

    await credentials.setActive(a.id)
    expect((await authStore.resolveAccess()).accessToken).toBe('token-a')
    await quota.probe()
    const oldObserver = quota.captureObserver()
    await credentials.setActive(b.id)
    expect(quota.latest()).toBeNull()
    expect(oldObserver({})).toBeNull()
    expect(quota.latest()).toBeNull()
    expect((await authStore.resolveAccess()).accessToken).toBe('token-b')
    expect((await credentials.getActive()).status).toMatchObject({
      status: 'authenticated',
      accountLabel: 'max',
    })
  } finally {
    await rm(stateDir, { recursive: true, force: true })
  }
})

test('injectableCliToken never injects keychain-sourced tokens', () => {
  expect(
    injectableCliToken({ accessToken: 'tok', source: 'keychain' })
  ).toBeNull()
  expect(injectableCliToken({ accessToken: 'tok', source: 'static' }))
    .toBe('tok')
  expect(injectableCliToken({ accessToken: 'tok', source: 'file' })).toBe('tok')
  expect(injectableCliToken({ accessToken: 'tok', source: 'env' })).toBe('tok')
})

test('buildClaudeEnv overlays CLAUDE_CODE_OAUTH_TOKEN', () => {
  const env = buildClaudeEnv({ PATH: '/bin' }, { oauthAccessToken: 'sk-test' })
  expect(env.CLAUDE_CODE_OAUTH_TOKEN).toBe('sk-test')
  expect(env.DISABLE_AUTO_COMPACT).toBe('1')
})

test('createClaudeCodeProvider wires credentials and auth status', async () => {
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-claude-prov-'))
  try {
    const provider = createClaudeCodeProvider({ stateDir })
    expect(provider.credentials).toBeDefined()
    await provider.credentials!.add!({
      accessToken: 'tok',
      subscriptionType: 'pro'
    })
    const status = await provider.auth!.status()
    expect(status).toMatchObject({
      status: 'authenticated',
      accountLabel: 'pro'
    })
  } finally {
    await rm(stateDir, { recursive: true, force: true })
  }
})

test('custom authStore skips credentials surface', () => {
  const provider = createClaudeCodeProvider({
    authStore: new StaticClaudeCodeAuthStore({
      accessToken: 'x',
      source: 'static'
    }),
  })
  expect(provider.credentials).toBeUndefined()
})

test('a pinned credential resolves whichever account is active', async () => {
  const pool = new MemoryCredentialPool()
  const credentials = createClaudeCodeCredentials(
    pool,
    new PoolAwareClaudeCodeAuthStore(pool)
  )
  const a = await credentials.add!({ accessToken: 'token-a' })
  const b = await credentials.add!({
    accessToken: 'token-b',
    subscriptionType: 'max'
  })
  await credentials.setActive(a.id)

  const pinned = new PoolAwareClaudeCodeAuthStore(pool, { credentialId: b.id })
  expect((await pinned.resolveAccess()).accessToken).toBe('token-b')
  expect(await pinned.status()).toMatchObject({
    status: 'authenticated',
    accountLabel: 'max',
  })
  expect(await pool.getActiveId()).toBe(a.id)
})

test('an empty pool is unauthenticated whatever the machine login is', async () => {
  const previous = process.env.CLAUDE_CODE_OAUTH_TOKEN
  process.env.CLAUDE_CODE_OAUTH_TOKEN = 'operator-token'
  try {
    const pool = new MemoryCredentialPool()
    const authStore = new PoolAwareClaudeCodeAuthStore(pool)
    // The message is the pool's own: neither the environment nor the keychain
    // lookup produced it.
    expect(await authStore.status()).toEqual({
      status: 'unauthenticated',
      message: 'No Claude Code account is signed in',
    })
    await expect(authStore.resolveAccess()).rejects.toThrow(
      'No Claude Code account is signed in'
    )

    const credentials = createClaudeCodeCredentials(pool, authStore)
    expect(credentials.capability()).toMatchObject({ canImportDefault: false })
    expect(credentials.importDefault).toBeUndefined()
    expect(await credentials.list()).toEqual([])
  } finally {
    if (previous === undefined)
      delete process.env.CLAUDE_CODE_OAUTH_TOKEN
    else
      process.env.CLAUDE_CODE_OAUTH_TOKEN = previous
  }
})

test('an empty pool with the vendor pool behind it stands for the environment token', async () => {
  const pool = fallbackCredentialPool(
    new MemoryCredentialPool(),
    claudeCodeVendorPool({
      env: { CLAUDE_CODE_OAUTH_TOKEN: 'own-token' },
      readKeychain: NO_KEYCHAIN,
    }),
  )
  const authStore = new PoolAwareClaudeCodeAuthStore(pool)
  const access = await authStore.resolveAccess()
  expect(access).toEqual({
    accessToken: 'own-token',
    source: 'env',
    subscriptionType: null,
    rateLimitTier: null,
  })
  expect(injectableCliToken(access)).toBe('own-token')
  expect(await pool.getActiveId()).toBe('env')
})

test('the keychain account is never injected into the CLI', async () => {
  const pool = claudeCodeVendorPool({ env: {}, readKeychain: KEYCHAIN_ITEM })
  expect(await pool.listMeta()).toMatchObject([
    { id: 'keychain', label: 'max', detail: 'tier-20x', source: 'vendor:keychain' },
  ])
  const access = await new PoolAwareClaudeCodeAuthStore(pool).resolveAccess()
  expect(access).toEqual({
    accessToken: 'keychain-token',
    source: 'keychain',
    subscriptionType: 'max',
    rateLimitTier: 'tier-20x',
  })
  expect(injectableCliToken(access)).toBeNull()
})

test('the environment token stands before the keychain item', async () => {
  const pool = claudeCodeVendorPool({
    env: { CLAUDE_CODE_OAUTH_TOKEN: 'own-token' },
    readKeychain: KEYCHAIN_ITEM,
  })
  expect((await pool.listMeta()).map((m) => m.id)).toEqual(['env', 'keychain'])
  expect(await pool.ensureActivePointer()).toBe('env')
  const pinned = new PoolAwareClaudeCodeAuthStore(pool, {
    credentialId: 'keychain'
  })
  expect((await pinned.resolveAccess()).source).toBe('keychain')
})

test('a malformed keychain item is an error, never a missing login', async () => {
  const pool = claudeCodeVendorPool({
    env: {},
    readKeychain: async () => JSON.stringify({ claudeAiOauth: {} }),
  })
  const authStore = new PoolAwareClaudeCodeAuthStore(pool)
  await expect(authStore.resolveAccess()).rejects.toThrow(/keychain credential is invalid/)
})

test('a machine without a vendor login has an empty vendor pool', async () => {
  const pool = claudeCodeVendorPool({ env: {}, readKeychain: NO_KEYCHAIN })
  expect(await pool.list()).toEqual([])
  expect(await pool.getActiveId()).toBeNull()
  expect(await pool.document('env').read()).toBeNull()
  expect(await new PoolAwareClaudeCodeAuthStore(pool).status()).toMatchObject({
    status: 'unauthenticated',
  })
})

test('the vendor pool refuses to be written to', async () => {
  const pool = claudeCodeVendorPool({
    env: { CLAUDE_CODE_OAUTH_TOKEN: 'own-token' },
    readKeychain: NO_KEYCHAIN,
  })
  const meta = (await pool.readMeta('env'))!
  await expect(pool.writeEntry(meta, '{}')).rejects.toThrow(CredentialPoolError)
  await expect(pool.remove('env')).rejects.toThrow(CredentialPoolError)
  await expect(pool.setActiveId('keychain')).rejects.toThrow(/not found/)
  await pool.setActiveId('env')
  expect(await pool.document('env').replace('{}', 'any')).toBe(false)
  expect(await pool.list()).toHaveLength(1)
})

test('a read-only document is never renewed, even when it has expired', async () => {
  const expired = JSON.stringify({
    accessToken: 'at-expired',
    refreshToken: 'rt-1',
    expiresAt: new Date(Date.now() - 60_000).toISOString(),
  })
  let refreshes = 0
  const refresh = async (secret: ClaudeCodeOAuthSecret) => {
    refreshes += 1
    return { ...secret, accessToken: 'at-renewed' }
  }
  // The same secret in a pool that stores it is due for renewal.
  const stored = new MemoryCredentialPool()
  await stored.writeEntry(
    { id: 'a', label: 'a', updatedAt: '2026-01-01T00:00:00.000Z' },
    expired
  )
  const renewing = new PoolAwareClaudeCodeAuthStore(stored, { refresh })
  expect((await renewing.resolveAccess()).accessToken).toBe('at-renewed')
  expect(refreshes).toBe(1)

  // The keychain item carries the CLI's refresh token and expiry; the vendor
  // document leaves them to the CLI.
  const vendor = claudeCodeVendorPool({
    env: {},
    readKeychain: async () => JSON.stringify({
      claudeAiOauth: {
        accessToken: 'at-expired',
        refreshToken: 'rt-1',
        expiresAt: Date.now() - 60_000,
      },
    }),
  })
  const authStore = new PoolAwareClaudeCodeAuthStore(vendor, { refresh })
  expect((await authStore.resolveAccess()).accessToken).toBe('at-expired')
  expect(refreshes).toBe(1)
  const text = (await vendor.document('keychain').read())!.text
  expect(text).not.toContain('rt-1')
})

test('importDefault copies the vendor login into the pool as a stored secret', async () => {
  const pool = new MemoryCredentialPool()
  const authStore = new PoolAwareClaudeCodeAuthStore(pool)
  const credentials = createClaudeCodeCredentials(pool, authStore, {
    importFrom: claudeCodeVendorPool({ env: {}, readKeychain: KEYCHAIN_ITEM }),
  })
  expect(credentials.capability()).toMatchObject({ canImportDefault: true })
  const info = await credentials.importDefault!()
  expect(info).toMatchObject({ label: 'max', detail: 'tier-20x' })
  expect(await pool.readMeta(info.id)).toMatchObject({ source: 'vendor:default' })
  expect(JSON.parse(pool.entries()[0]!.secretText)).toEqual({
    accessToken: 'keychain-token',
    subscriptionType: 'max',
    rateLimitTier: 'tier-20x',
  })
  expect(await authStore.resolveAccess()).toMatchObject({
    accessToken: 'keychain-token',
    source: 'file',
  })

  const none = createClaudeCodeCredentials(pool, authStore, {
    importFrom: claudeCodeVendorPool({ env: {}, readKeychain: NO_KEYCHAIN }),
  })
  await expect(none.importDefault!()).rejects.toThrow(/No Claude Code OAuth to import/)
})

test('the default provider stands for the vendor login until an account is stored', async () => {
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-claude-vendor-'))
  try {
    const provider = createClaudeCodeProvider({
      stateDir,
      vendor: {
        env: { CLAUDE_CODE_OAUTH_TOKEN: 'own-token' },
        readKeychain: NO_KEYCHAIN,
      },
    })
    expect(provider.credentials!.capability()).toMatchObject({
      canImportDefault: true
    })
    expect((await provider.credentials!.getActive()).credentialId).toBe('env')
    const added = await provider.credentials!.add!({
      accessToken: 'tok',
      subscriptionType: 'pro'
    })
    expect(await provider.credentials!.getActive()).toMatchObject({
      credentialId: added.id,
      status: { status: 'authenticated', accountLabel: 'pro' },
    })

    const owned = createClaudeCodeProvider({
      credentialPool: new MemoryCredentialPool(),
      vendor: {
        env: { CLAUDE_CODE_OAUTH_TOKEN: 'own-token' },
        readKeychain: NO_KEYCHAIN,
      },
    })
    expect(owned.credentials!.capability()).toMatchObject({
      canImportDefault: false
    })
    expect(await owned.auth!.status()).toMatchObject({
      status: 'unauthenticated'
    })
  } finally {
    await rm(stateDir, { recursive: true, force: true })
  }
})

test('pinned credentials keep their quota when the active account changes', async () => {
  const pool = new MemoryCredentialPool()
  const quota = createProviderQuota({
    providerId: 'claude-code',
    canProbe: true,
    probe: async () => ({ accountLabel: 'pinned-account', windows: [] }),
  })
  let changes = 0
  const credentials = createClaudeCodeCredentials(
    pool,
    new PoolAwareClaudeCodeAuthStore(pool, { credentialId: 'pinned' }),
    { quota, pinned: true, onActiveChange: () => { changes += 1 } },
  )
  const a = await credentials.add!({ accessToken: 'token-a' })
  await quota.probe()
  expect(quota.latest()).not.toBeNull()

  await credentials.setActive(a.id)
  expect(quota.latest()).not.toBeNull()
  await credentials.remove!(a.id)
  expect(quota.latest()).not.toBeNull()
  expect(changes).toBe(3)
})

test('createClaudeCodeProvider stands for the pinned account of an injected pool', async () => {
  const pool = new MemoryCredentialPool()
  const shared = createClaudeCodeProvider({ credentialPool: pool })
  const a = await shared.credentials!.add!({
    accessToken: 'token-a',
    subscriptionType: 'pro'
  })
  const b = await shared.credentials!.add!({
    accessToken: 'token-b',
    subscriptionType: 'max'
  })
  expect((await shared.credentials!.getActive()).credentialId).toBe(a.id)

  const pinned = createClaudeCodeProvider({
    credentialPool: pool,
    credentialId: b.id
  })
  expect(await pinned.auth!.status()).toMatchObject({
    status: 'authenticated',
    accountLabel: 'max',
  })
  expect(await shared.auth!.status()).toMatchObject({ accountLabel: 'pro' })
})
