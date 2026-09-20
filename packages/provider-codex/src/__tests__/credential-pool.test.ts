import { expect, test } from 'bun:test'
import { createProviderQuota } from '@demicodes/provider'
import {
  fallbackCredentialPool,
  MemoryCredentialPool
} from '@demicodes/provider/credentials-pool'
import { Buffer } from 'node:buffer'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { CodexDocumentAuthStore } from '../auth'
import { codexVendorPool } from '../vendor'
import {
  createCodexCredentials,
  PoolAwareCodexAuthStore
} from '../credentials'

function jwt(payload: Record<string, unknown>): string {
  const header = Buffer.from(JSON.stringify({ alg: 'none', typ: 'JWT' }))
    .toString('base64url')
  const body = Buffer.from(JSON.stringify(payload)).toString('base64url')
  return `${header}.${body}.sig`
}

function authText(email: string, accountId: string, refreshToken = 'refresh'): string {
  const claims = {
    email,
    'https://api.openai.com/auth': { chatgpt_account_id: accountId },
  }
  return `${JSON.stringify({
    auth_mode: 'chatgpt',
    tokens: {
      id_token: jwt(claims),
      access_token: jwt({ exp: 1_900_000_000, ...claims }),
      refresh_token: refreshToken,
      account_id: accountId,
    },
    last_refresh: new Date().toISOString(),
  })}\n`
}

const meta = (id: string) => ({
  id,
  label: `${id}@example.com`,
  identityKey: id,
  updatedAt: new Date().toISOString()
})

async function poolOf(...ids: string[]): Promise<MemoryCredentialPool> {
  const pool = new MemoryCredentialPool()
  for (const id of ids)
    await pool.writeEntry(meta(id), authText(`${id}@example.com`, id))
  return pool
}

test('a store built for one account resolves it, whichever account is active', async () => {
  const pool = await poolOf('a', 'b')
  await pool.setActiveId('a')
  const pinned = await new PoolAwareCodexAuthStore(pool, { credentialId: 'b' })
    .resolveAuth()
  const following = await new PoolAwareCodexAuthStore(pool).resolveAuth()
  expect(pinned.kind === 'chatgpt' && pinned.accountId).toBe('b')
  expect(following.kind === 'chatgpt' && following.accountId).toBe('a')
})

test('a refresh that loses the write uses the tokens the winner stored', async () => {
  const pool = await poolOf('a')
  const store = new CodexDocumentAuthStore({
    document: pool.document('a'),
    refresh: async () => {
      // Another worker refreshes and stores first.
      await pool.writeEntry(meta('a'), authText('a@example.com', 'a', 'winner'))
      return { access_token: jwt({ exp: 1_900_000_000 }), refresh_token: 'loser' }
    },
  })
  const auth = await store.resolveAuth({ forceRefresh: true })
  expect(auth.kind === 'chatgpt' && auth.refreshToken).toBe('winner')
  expect((await pool.document('a').read())?.text).toContain('winner')
})

test('a refresh the vendor refuses uses the stored tokens when someone else refreshed, and fails when nobody did', async () => {
  const pool = await poolOf('a')
  const raced = new CodexDocumentAuthStore({
    document: pool.document('a'),
    refresh: async () => {
      await pool.writeEntry(meta('a'), authText('a@example.com', 'a', 'winner'))
      throw new Error('refresh token already used')
    },
  })
  const auth = await raced.resolveAuth({ forceRefresh: true })
  expect(auth.kind === 'chatgpt' && auth.refreshToken).toBe('winner')

  const alone = new CodexDocumentAuthStore({
    document: pool.document('a'),
    refresh: async () => {
      throw new Error('refresh token revoked')
    },
  })
  await expect(alone.resolveAuth({ forceRefresh: true }))
    .rejects.toThrow('refresh token revoked')
})

test('refreshes of one account take turns: the later one spends the earlier one\'s token', async () => {
  const pool = await poolOf('a')
  const spent: string[] = []
  const refresh = async (token: string) => {
    spent.push(token)
    await Bun.sleep(5)
    return {
      access_token: jwt({ exp: 1_900_000_000 }),
      refresh_token: `after-${token}`
    }
  }
  await Promise.all([
    new CodexDocumentAuthStore({ document: pool.document('a'), refresh })
      .resolveAuth({ forceRefresh: true }),
    new CodexDocumentAuthStore({ document: pool.document('a'), refresh })
      .resolveAuth({ forceRefresh: true }),
  ])
  expect(spent).toEqual(['refresh', 'after-refresh'])
})

test('an empty pool is unauthenticated whatever the machine holds; only a pool composed with the vendor login stands for it', async () => {
  const codexHome = await mkdtemp(join(tmpdir(), 'demi-codex-home-'))
  try {
    await writeFile(join(codexHome, 'auth.json'), authText('operator@example.com', 'operator'))
    const pool = new MemoryCredentialPool()
    const store = new PoolAwareCodexAuthStore(pool)
    expect((await store.status()).status).toBe('unauthenticated')
    await expect(store.resolveAuth()).rejects.toThrow('No Codex account')
    const credentials = createCodexCredentials(pool, store)
    expect(credentials.capability()).toMatchObject({ canImportDefault: false })
    expect(credentials.importDefault).toBeUndefined()

    // The kit's default for a local user: the same store over a composed pool.
    const vendor = codexVendorPool({ codexHome })
    const composed = fallbackCredentialPool(new MemoryCredentialPool(), vendor)
    const own = new PoolAwareCodexAuthStore(composed)
    expect(await own.status()).toMatchObject({
      status: 'authenticated',
      accountLabel: 'operator@example.com'
    })
    expect((await composed.list()).map(account => account.label))
      .toEqual(['operator@example.com'])
    // The first stored account takes over, and the vendor login is left as it was.
    const imported = await createCodexCredentials(composed, own, { importFrom: vendor })
      .importDefault!()
    expect((await composed.list()).map(account => account.id)).toEqual([imported.id])
    await expect(vendor.writeEntry(meta('x'), '{}')).rejects.toThrow('vendor')
  } finally {
    await rm(codexHome, { recursive: true, force: true })
  }
})

test('a provider standing for one account keeps its usage when another account is selected or removed', async () => {
  const quotaOf = () => {
    const quota = createProviderQuota({
      providerId: 'codex',
      canProbe: true,
      probeCost: 'free',
      probe: async () => ({ windows: [{ id: 'weekly', usedPercent: 10, resetsAt: null }] }),
    })
    return quota
  }
  for (const pinned of [true, false]) {
    const pool = await poolOf('a', 'b')
    await pool.setActiveId('a')
    const quota = quotaOf()
    await quota.probe({ force: true })
    const credentials = createCodexCredentials(
      pool,
      new PoolAwareCodexAuthStore(pool, pinned ? { credentialId: 'a' } : {}),
      { quota, pinned }
    )
    await credentials.setActive('b')
    expect(quota.latest() !== null).toBe(pinned)
  }
})

test('the framework\'s file pool keeps a refreshed account in its entry file, not the vendor home', async () => {
  const { readFile } = await import('node:fs/promises')
  const { openCodexCredentialPool } = await import('../credentials')
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-codex-state-'))
  const codexHome = await mkdtemp(join(tmpdir(), 'demi-codex-home-'))
  try {
    const pool = openCodexCredentialPool({ stateDir })
    await pool.writeEntry(meta('a'), authText('a@example.com', 'a'))
    const store = new PoolAwareCodexAuthStore(pool, {
      refresh: async () => ({
        access_token: jwt({ exp: 1_900_000_000 }),
        refresh_token: 'renewed'
      }),
    })
    const auth = await store.resolveAuth({ forceRefresh: true })
    expect(auth.kind === 'chatgpt' && auth.refreshToken).toBe('renewed')
    expect(await readFile(pool.secretPath('a'), 'utf8')).toContain('renewed')
    // A rebuilt store reads what the refresh kept.
    const again = await new PoolAwareCodexAuthStore(pool).resolveAuth()
    expect(again.kind === 'chatgpt' && again.refreshToken).toBe('renewed')
  } finally {
    await rm(stateDir, { recursive: true, force: true })
    await rm(codexHome, { recursive: true, force: true })
  }
})

test('a vendor login that cannot be read is reported as an error, not thrown', async () => {
  const codexHome = await mkdtemp(join(tmpdir(), 'demi-codex-home-'))
  try {
    await writeFile(join(codexHome, 'auth.json'), '{not json')
    const status = await new PoolAwareCodexAuthStore(
      fallbackCredentialPool(new MemoryCredentialPool(), codexVendorPool({ codexHome }))
    ).status()
    expect(status.status).toBe('error')
  } finally {
    await rm(codexHome, { recursive: true, force: true })
  }
})
