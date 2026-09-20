import { expect, test } from 'bun:test'
import {
  type CredentialDocument,
  MemoryCredentialPool,
} from '@demicodes/provider/credentials-pool'
import { ClaudeCodeAuthError, FileClaudeCodeAuthStore } from '../auth'
import type { ClaudeCodeOAuthSecret } from '../secret'

const META = {
  id: 'oauth',
  label: 'oauth',
  updatedAt: '2026-01-01T00:00:00.000Z',
}

/**
 * Runs `body` with a synthetic oauth secret held in a pool document, and with
 * `storeElsewhere`, which stores a text as another writer would.
 */
async function withOAuthDocument(
  contents: string,
  body: (
    document: CredentialDocument,
    storeElsewhere: (text: string) => Promise<void>,
  ) => Promise<void>,
): Promise<void> {
  const pool = new MemoryCredentialPool()
  await pool.writeEntry(META, contents)
  await body(pool.document(META.id), async (text) => {
    await pool.writeEntry(META, text)
  })
}

function secretJson(secret: Partial<ClaudeCodeOAuthSecret>): string {
  return JSON.stringify(secret)
}

test('a valid oauth document resolves to file-sourced access', async () => {
  await withOAuthDocument(
    secretJson({ accessToken: 'at-1', subscriptionType: 'max' }),
    async (document) => {
      const store = new FileClaudeCodeAuthStore({ document })
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

test('a corrupt oauth document is an error, never a repaired value', async () => {
  await withOAuthDocument('{not json', async (document) => {
    const store = new FileClaudeCodeAuthStore({ document })
    await expect(store.resolveAccess()).rejects.toThrow(/not valid JSON/)
  })
  await withOAuthDocument(secretJson({ subscriptionType: 'max' }), async (document) => {
    const store = new FileClaudeCodeAuthStore({ document })
    await expect(store.resolveAccess()).rejects.toThrow(/accessToken/)
    expect(await store.status()).toMatchObject({ status: 'error' })
  })
})

test('a missing oauth document reads as an unauthenticated credential', async () => {
  const store = new FileClaudeCodeAuthStore({
    document: new MemoryCredentialPool().document('absent'),
  })
  await expect(store.resolveAccess()).rejects.toThrow(ClaudeCodeAuthError)
  expect(await store.status()).toMatchObject({ status: 'unauthenticated' })
})

test('an expiring secret is renewed and the renewal is written back', async () => {
  const expiresAt = new Date(Date.now() + 60_000).toISOString()
  await withOAuthDocument(
    secretJson({ accessToken: 'at-old', refreshToken: 'rt-1', expiresAt }),
    async (document) => {
      const store = new FileClaudeCodeAuthStore({
        document,
        refresh: async (secret) => ({
          ...secret,
          accessToken: 'at-new',
          expiresAt: new Date(Date.now() + 3_600_000).toISOString(),
        }),
      })
      expect((await store.resolveAccess()).accessToken).toBe('at-new')
      const written: unknown = JSON.parse((await document.read())!.text)
      expect(written).toMatchObject({
        accessToken: 'at-new',
        refreshToken: 'rt-1'
      })
    },
  )
})

test('an invalid renewal is rejected before it reaches the document', async () => {
  const expiresAt = new Date(Date.now() + 60_000).toISOString()
  const stored = secretJson({
    accessToken: 'at-old',
    refreshToken: 'rt-1',
    expiresAt
  })
  await withOAuthDocument(stored, async (document) => {
    const store = new FileClaudeCodeAuthStore({
      document,
      refresh: async () => ({ accessToken: '' }),
    })
    await expect(store.resolveAccess()).rejects.toThrow(/renewal is invalid/)
    expect((await document.read())?.text).toBe(stored)
  })
})

test('a secret that is not expiring is used as it is', async () => {
  const expiresAt = new Date(Date.now() + 3_600_000).toISOString()
  await withOAuthDocument(
    secretJson({ accessToken: 'at-old', refreshToken: 'rt-1', expiresAt }),
    async (document) => {
      let refreshes = 0
      const store = new FileClaudeCodeAuthStore({
        document,
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

test('a renewal whose replace loses uses the stored winner', async () => {
  const expiresAt = new Date(Date.now() + 60_000).toISOString()
  await withOAuthDocument(
    secretJson({ accessToken: 'at-old', refreshToken: 'rt-1', expiresAt }),
    async (document, storeElsewhere) => {
      const winner = secretJson({ accessToken: 'at-winner', refreshToken: 'rt-2' })
      const store = new FileClaudeCodeAuthStore({
        document,
        refresh: async (secret) => {
          // Another process stores its renewal while this one is in flight.
          await storeElsewhere(winner)
          return { ...secret, accessToken: 'at-loser' }
        },
      })
      expect((await store.resolveAccess()).accessToken).toBe('at-winner')
      expect((await document.read())?.text).toBe(winner)
    },
  )
})

test('a refused renewal uses the tokens stored meanwhile', async () => {
  const expiresAt = new Date(Date.now() + 60_000).toISOString()
  await withOAuthDocument(
    secretJson({ accessToken: 'at-old', refreshToken: 'rt-1', expiresAt }),
    async (document, storeElsewhere) => {
      const store = new FileClaudeCodeAuthStore({
        document,
        refresh: async () => {
          // The refresh token was spent by whoever stored these tokens.
          await storeElsewhere(
            secretJson({ accessToken: 'at-winner', refreshToken: 'rt-2' })
          )
          throw new Error('invalid_grant')
        },
      })
      expect((await store.resolveAccess()).accessToken).toBe('at-winner')
    },
  )
})

test('a refused renewal of an unchanged document is an error', async () => {
  const expiresAt = new Date(Date.now() + 60_000).toISOString()
  await withOAuthDocument(
    secretJson({ accessToken: 'at-old', refreshToken: 'rt-1', expiresAt }),
    async (document) => {
      const store = new FileClaudeCodeAuthStore({
        document,
        refresh: async () => {
          throw new Error('invalid_grant')
        },
      })
      await expect(store.resolveAccess()).rejects.toThrow(/invalid_grant/)
    },
  )
})
