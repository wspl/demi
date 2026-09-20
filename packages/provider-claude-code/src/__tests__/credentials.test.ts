import { expect, test } from 'bun:test'
import { createProviderQuota } from '@demicodes/provider'
import { MemoryCredentialPool } from '@demicodes/provider/credentials-pool'
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

test('an empty pool without a vendor default never reads the machine login', async () => {
  const previous = process.env.CLAUDE_CODE_OAUTH_TOKEN
  process.env.CLAUDE_CODE_OAUTH_TOKEN = 'operator-token'
  try {
    const pool = new MemoryCredentialPool(false)
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
    await expect(credentials.importDefault!()).rejects.toThrow(/vendor login/)
    expect(await credentials.list()).toEqual([])
  } finally {
    if (previous === undefined)
      delete process.env.CLAUDE_CODE_OAUTH_TOKEN
    else
      process.env.CLAUDE_CODE_OAUTH_TOKEN = previous
  }
})

test('an empty pool with a vendor default reads the environment token', async () => {
  const previous = process.env.CLAUDE_CODE_OAUTH_TOKEN
  process.env.CLAUDE_CODE_OAUTH_TOKEN = 'own-token'
  try {
    const pool = new MemoryCredentialPool(true)
    const authStore = new PoolAwareClaudeCodeAuthStore(pool)
    expect(await authStore.resolveAccess()).toEqual({
      accessToken: 'own-token',
      source: 'env',
    })
  } finally {
    if (previous === undefined)
      delete process.env.CLAUDE_CODE_OAUTH_TOKEN
    else
      process.env.CLAUDE_CODE_OAUTH_TOKEN = previous
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
