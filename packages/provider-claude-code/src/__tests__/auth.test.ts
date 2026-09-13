import { expect, test } from 'bun:test'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { ClaudeCodeAuthError, FileClaudeCodeAuthStore } from '../auth'
import type { ClaudeCodeOAuthSecret } from '../secret'

/** Runs `body` with a synthetic oauth.json in a throwaway directory. */
async function withOAuthFile(
  contents: string,
  body: (path: string) => Promise<void>,
): Promise<void> {
  const dir = await mkdtemp(join(tmpdir(), 'demi-claude-auth-'))
  const path = join(dir, 'oauth.json')
  await writeFile(path, contents)
  try {
    await body(path)
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
}

function secretJson(secret: Partial<ClaudeCodeOAuthSecret>): string {
  return JSON.stringify(secret)
}

test('a valid oauth file resolves to file-sourced access', async () => {
  await withOAuthFile(
    secretJson({ accessToken: 'at-1', subscriptionType: 'max' }),
    async (path) => {
      const store = new FileClaudeCodeAuthStore({ oauthFile: path })
      expect(await store.resolveAccess()).toEqual({
        accessToken: 'at-1',
        source: 'file',
        subscriptionType: 'max',
        rateLimitTier: null,
      })
      expect(await store.status()).toMatchObject({
        status: 'authenticated',
        accountLabel: 'max',
      })
    },
  )
})

test('a corrupt oauth file is an error, never a repaired value', async () => {
  await withOAuthFile('{not json', async (path) => {
    const store = new FileClaudeCodeAuthStore({ oauthFile: path })
    await expect(store.resolveAccess()).rejects.toThrow(/not valid JSON/)
  })
  await withOAuthFile(secretJson({ subscriptionType: 'max' }), async (path) => {
    const store = new FileClaudeCodeAuthStore({ oauthFile: path })
    await expect(store.resolveAccess()).rejects.toThrow(/accessToken/)
    expect(await store.status()).toMatchObject({ status: 'error' })
  })
})

test('a missing oauth file reads as an unauthenticated credential', async () => {
  const store = new FileClaudeCodeAuthStore({
    oauthFile: join(tmpdir(), 'demi-claude-auth-absent', 'oauth.json'),
  })
  await expect(store.resolveAccess()).rejects.toThrow(ClaudeCodeAuthError)
  expect(await store.status()).toMatchObject({ status: 'unauthenticated' })
})

test('an expiring secret is renewed and the renewal is written back', async () => {
  const expiresAt = new Date(Date.now() + 60_000).toISOString()
  await withOAuthFile(
    secretJson({ accessToken: 'at-old', refreshToken: 'rt-1', expiresAt }),
    async (path) => {
      const store = new FileClaudeCodeAuthStore({
        oauthFile: path,
        refresh: async (secret) => ({
          ...secret,
          accessToken: 'at-new',
          expiresAt: new Date(Date.now() + 3_600_000).toISOString(),
        }),
      })
      expect((await store.resolveAccess()).accessToken).toBe('at-new')
      const written: unknown = JSON.parse(await readFile(path, 'utf8'))
      expect(written).toMatchObject({
        accessToken: 'at-new',
        refreshToken: 'rt-1'
      })
    },
  )
})

test('an invalid renewal is rejected before it reaches the file', async () => {
  const expiresAt = new Date(Date.now() + 60_000).toISOString()
  const stored = secretJson({
    accessToken: 'at-old',
    refreshToken: 'rt-1',
    expiresAt
  })
  await withOAuthFile(stored, async (path) => {
    const store = new FileClaudeCodeAuthStore({
      oauthFile: path,
      refresh: async () => ({ accessToken: '' }),
    })
    await expect(store.resolveAccess()).rejects.toThrow(/renewal is invalid/)
    expect(await readFile(path, 'utf8')).toBe(stored)
  })
})

test('a secret that is not expiring is used as it is', async () => {
  const expiresAt = new Date(Date.now() + 3_600_000).toISOString()
  await withOAuthFile(
    secretJson({ accessToken: 'at-old', refreshToken: 'rt-1', expiresAt }),
    async (path) => {
      let refreshes = 0
      const store = new FileClaudeCodeAuthStore({
        oauthFile: path,
        refresh: async (secret) => {
          refreshes += 1
          return secret
        },
      })
      expect((await store.resolveAccess()).accessToken).toBe('at-old')
      expect(refreshes).toBe(0)
    },
  )
})
