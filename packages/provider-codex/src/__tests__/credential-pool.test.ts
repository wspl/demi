import { expect, test } from 'bun:test'
import { createProviderQuota } from '@demicodes/provider'
import { MemoryCredentialPool } from '@demicodes/provider/credentials-pool'
import { Buffer } from 'node:buffer'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { FileCodexAuthStore } from '../auth'
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
  const store = new FileCodexAuthStore({
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
  const raced = new FileCodexAuthStore({
    document: pool.document('a'),
    refresh: async () => {
      await pool.writeEntry(meta('a'), authText('a@example.com', 'a', 'winner'))
      throw new Error('refresh token already used')
    },
  })
  const auth = await raced.resolveAuth({ forceRefresh: true })
  expect(auth.kind === 'chatgpt' && auth.refreshToken).toBe('winner')

  const alone = new FileCodexAuthStore({
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
    new FileCodexAuthStore({ document: pool.document('a'), refresh })
      .resolveAuth({ forceRefresh: true }),
    new FileCodexAuthStore({ document: pool.document('a'), refresh })
      .resolveAuth({ forceRefresh: true }),
  ])
  expect(spent).toEqual(['refresh', 'after-refresh'])
})

test('a pool that lends no vendor login is unauthenticated when empty, whatever the machine holds', async () => {
  const codexHome = await mkdtemp(join(tmpdir(), 'demi-codex-home-'))
  try {
    await writeFile(join(codexHome, 'auth.json'), authText('operator@example.com', 'operator'))
    const pool = new MemoryCredentialPool(false)
    const store = new PoolAwareCodexAuthStore(pool, { codexHome })
    expect((await store.status()).status).toBe('unauthenticated')
    await expect(store.resolveAuth()).rejects.toThrow('No Codex account')
    const credentials = createCodexCredentials(pool, store, { codexHome })
    expect(credentials.capability()).toMatchObject({ canImportDefault: false })
    await expect(credentials.importDefault!()).rejects.toThrow('vendor login')
    // The framework's own pool still stands for the user's vendor login.
    const own = new PoolAwareCodexAuthStore(new MemoryCredentialPool(true), { codexHome })
    expect((await own.status()).status).toBe('authenticated')
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
