import { expect, test } from 'bun:test'
import { Buffer } from 'node:buffer'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { FileGrokAuthStore, isAbandonedGrokAuthLock, selectAuthEntry, redactSecretText } from '../auth'

test('FileGrokAuthStore resolves OIDC session from Grok CLI auth.json shape', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-grok-auth-'))
  try {
    const access = jwt({ exp: 1_900_000_000, email: 'user@example.com' })
    const entryKey = 'https://auth.x.ai::client-1'
    await writeFile(
      join(dir, 'auth.json'),
      JSON.stringify({
        [entryKey]: {
          key: access,
          auth_mode: 'oidc',
          refresh_token: 'refresh-secret',
          expires_at: '2030-01-01T00:00:00.000Z',
          oidc_issuer: 'https://auth.x.ai',
          oidc_client_id: 'client-1',
          email: 'user@example.com',
        },
      }),
    )

    const store = new FileGrokAuthStore({ grokHome: dir, now: () => new Date('2026-06-19T00:00:00.000Z') })
    const auth = await store.resolveAuth()

    expect(auth.accessToken).toBe(access)
    expect(auth.email).toBe('user@example.com')
    expect(auth.entryKey).toBe(entryKey)
    expect(auth.clientId).toBe('client-1')
    expect(await store.status()).toEqual({ status: 'authenticated', accountLabel: 'user@example.com' })
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('FileGrokAuthStore refreshes near-expiry OIDC tokens and preserves sibling entries', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-grok-refresh-'))
  const now = new Date('2026-06-19T00:00:00.000Z')
  const oldAccess = jwt({ exp: Math.floor((now.getTime() + 30_000) / 1000) })
  const newAccess = jwt({ exp: Math.floor((now.getTime() + 3_600_000) / 1000) })
  const entryKey = 'https://auth.x.ai::client-1'
  try {
    await writeFile(
      join(dir, 'auth.json'),
      `${JSON.stringify({
        other: { key: 'leave-me', note: true },
        [entryKey]: {
          key: oldAccess,
          auth_mode: 'oidc',
          refresh_token: 'refresh-old',
          expires_at: new Date(now.getTime() + 30_000).toISOString(),
          oidc_issuer: 'https://auth.x.ai',
          oidc_client_id: 'client-1',
          email: 'before@example.com',
          custom_future_field: { keep: true },
        },
      })}\n`,
      { mode: 0o600 },
    )

    const store = new FileGrokAuthStore({
      grokHome: dir,
      now: () => now,
      refresh: async (input) => {
        expect(input.refreshToken).toBe('refresh-old')
        expect(input.clientId).toBe('client-1')
        expect(input.tokenEndpoint).toBe('https://auth.x.ai/oauth2/token')
        return {
          access_token: newAccess,
          refresh_token: 'refresh-new',
          expires_in: 3600,
        }
      },
    })
    const auth = await store.resolveAuth()
    const written = JSON.parse(await readFile(join(dir, 'auth.json'), 'utf8'))

    expect(auth.accessToken).toBe(newAccess)
    expect(written.other).toEqual({ key: 'leave-me', note: true })
    expect(written[entryKey].refresh_token).toBe('refresh-new')
    expect(written[entryKey].custom_future_field).toEqual({ keep: true })
    expect(written[entryKey].key).toBe(newAccess)
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('FileGrokAuthStore reports missing auth without leaking secrets', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-grok-missing-'))
  try {
    const store = new FileGrokAuthStore({ grokHome: dir })
    expect(await store.status()).toMatchObject({ status: 'unauthenticated' })
    expect(redactSecretText('Bearer super-secret-token-value')).toBe('Bearer [REDACTED]')
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('selectAuthEntry prefers OIDC entries on auth.x.ai', () => {
  const selected = selectAuthEntry({
    'https://other.example::a': { key: 'a', auth_mode: 'api_key' },
    'https://auth.x.ai::cli': {
      key: 'b',
      auth_mode: 'oidc',
      refresh_token: 'r',
      oidc_issuer: 'https://auth.x.ai',
    },
  })
  expect(selected?.entryKey).toBe('https://auth.x.ai::cli')
})

test('FileGrokAuthStore steals abandoned Grok CLI auth.json.lock and refreshes', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-grok-stale-lock-'))
  const now = new Date('2026-06-19T00:00:00.000Z')
  const oldAccess = jwt({ exp: Math.floor((now.getTime() + 30_000) / 1000) })
  const newAccess = jwt({ exp: Math.floor((now.getTime() + 3_600_000) / 1000) })
  const entryKey = 'https://auth.x.ai::client-1'
  try {
    await writeFile(
      join(dir, 'auth.json'),
      JSON.stringify({
        [entryKey]: {
          key: oldAccess,
          auth_mode: 'oidc',
          refresh_token: 'refresh-old',
          expires_at: new Date(now.getTime() + 30_000).toISOString(),
          oidc_issuer: 'https://auth.x.ai',
          oidc_client_id: 'client-1',
        },
      }),
    )
    // Dead pid + old timestamp — the shape Grok CLI leaves behind after a crash.
    await writeFile(join(dir, 'auth.json.lock'), '999999:1000')

    const store = new FileGrokAuthStore({
      grokHome: dir,
      now: () => now,
      refresh: async () => ({ access_token: newAccess, refresh_token: 'refresh-new', expires_in: 3600 }),
    })
    const auth = await store.resolveAuth()
    expect(auth.accessToken).toBe(newAccess)
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('a live Grok auth lock is never abandoned because of age alone', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-grok-live-lock-'))
  const lockFile = join(dir, 'auth.json.lock')
  try {
    await writeFile(lockFile, `${process.pid}:1000`)
    expect(await isAbandonedGrokAuthLock(lockFile, new Date('2026-06-19T00:00:00.000Z'))).toBe(false)
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

function jwt(payload: Record<string, unknown>): string {
  const header = Buffer.from(JSON.stringify({ alg: 'none', typ: 'JWT' })).toString('base64url')
  const body = Buffer.from(JSON.stringify(payload)).toString('base64url')
  return `${header}.${body}.sig`
}
