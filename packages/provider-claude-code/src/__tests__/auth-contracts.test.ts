import { expect, test } from 'bun:test'
import { mkdtemp, mkdir, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { errorCode } from '@demicodes/utils'
import { ClaudeCodeAuthError, FileClaudeCodeAuthStore } from '../auth'
import { createClaudeCodeCredentials, openClaudeCodeCredentialPool } from '../credentials'
import { resolveAccessFromStore } from '../oauth'
import { refreshClaudeCodeSecret, runClaudeCodeLogin } from '../login'

const malformedSecrets = [
  null, [], {}, { accessToken: 42 }, { accessToken: '' }, { accessToken: '  ' },
  { access_token: 'snake-case-is-not-the-pool-contract' },
  { accessToken: 'token', refreshToken: false },
  { accessToken: 'token', expiresAt: 42 }, { accessToken: 'token', expiresAt: 'yesterday' },
  { accessToken: 'token', scopes: 'user:inference' }, { accessToken: 'token', scopes: [42] },
  { accessToken: 'token', subscriptionType: {} }, { accessToken: 'token', rateLimitTier: [] },
  { accessToken: 'token', emailAddress: false },
]

function fakeFetch(body: unknown): typeof fetch {
  return Object.assign(async () => Response.json(body), { preconnect: () => {} })
}

test('Claude OAuth file and add use the same secret contract and reject bad optional fields', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'claude-secret-contract-'))
  try {
    const oauthFile = join(dir, 'oauth.json')
    const store = new FileClaudeCodeAuthStore({ oauthFile, env: {}, readKeychain: async () => null })
    const pool = openClaudeCodeCredentialPool({ stateDir: join(dir, 'pool') })
    const credentials = createClaudeCodeCredentials(pool, store)
    for (const value of malformedSecrets) {
      await writeFile(oauthFile, JSON.stringify(value))
      await expect(store.resolveAccess()).rejects.toMatchObject({ code: 'auth_invalid' })
      await expect(credentials.add!({ oauth: value })).rejects.toMatchObject({ code: 'auth_invalid' })
    }
    expect(await pool.list()).toEqual([])
    const secret = {
      accessToken: 'token', refreshToken: 'refresh', expiresAt: null,
      scopes: ['user:inference'], subscriptionType: null, rateLimitTier: null,
    }
    const entry = await credentials.add!({ oauth: secret })
    expect(JSON.parse(await readFile(pool.secretPath(entry.id), 'utf8'))).toEqual(secret)
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('only missing OAuth state may return null; IO and corruption propagate to import callers', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'claude-auth-errors-'))
  try {
    const oauthFile = join(dir, 'oauth.json')
    const store = new FileClaudeCodeAuthStore({ oauthFile, env: {}, readKeychain: async () => null })
    expect(await resolveAccessFromStore(store)).toBeNull()
    await mkdir(oauthFile)
    try {
      await resolveAccessFromStore(store)
      throw new Error('Expected directory read failure')
    } catch (error) {
      expect(errorCode(error)).toBe('EISDIR')
    }
    await rm(oauthFile, { recursive: true })
    await writeFile(oauthFile, 'SYNTHETIC_SECRET_BAD_JSON')
    await expect(resolveAccessFromStore(store)).rejects.toMatchObject({ code: 'auth_invalid' })
    const credentials = createClaudeCodeCredentials(openClaudeCodeCredentialPool({ stateDir: dir }), store, {
      resolveDefaultAccess: () => store.resolveAccess(),
    })
    await expect(credentials.importDefault!()).rejects.toMatchObject({ code: 'auth_invalid' })
    const status = await store.status()
    expect(status.status).toBe('error')
    expect(JSON.stringify(status)).not.toContain('SYNTHETIC_SECRET')
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('keychain and environment inputs validate before resolving access without real credential access', async () => {
  for (const value of [
    [], { claudeAiOauth: [] }, { claudeAiOauth: { accessToken: 42 } },
    { claudeAiOauth: { accessToken: 'token', expiresAt: 'tomorrow' } },
    { claudeAiOauth: { accessToken: 'token', scopes: [42] } },
  ]) {
    const store = new FileClaudeCodeAuthStore({ env: {}, readKeychain: async () => JSON.stringify(value) })
    await expect(store.resolveAccess()).rejects.toMatchObject({ code: 'auth_invalid' })
  }
  const keychainStore = new FileClaudeCodeAuthStore({ env: {}, readKeychain: async () => JSON.stringify({
    claudeAiOauth: { accessToken: 'token', expiresAt: 1_900_000_000_000, scopes: [], subscriptionType: 'max' },
  }) })
  expect(await keychainStore.resolveAccess()).toMatchObject({ source: 'keychain', accessToken: 'token', subscriptionType: 'max' })
  const badEnv = new FileClaudeCodeAuthStore({ env: { CLAUDE_CODE_OAUTH_TOKEN: ' ' }, readKeychain: async () => {
    throw new Error('must not fall through to keychain')
  } })
  await expect(badEnv.resolveAccess()).rejects.toMatchObject({ code: 'auth_invalid' })
  const keychainFailure = new Error('keychain IO failed')
  const failed = new FileClaudeCodeAuthStore({ env: {}, readKeychain: async () => { throw keychainFailure } })
  await expect(resolveAccessFromStore(failed)).rejects.toBe(keychainFailure)
})

test('refresh validates before atomic replacement and supports explicit forceRefresh', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'claude-refresh-contract-'))
  try {
    const oauthFile = join(dir, 'oauth.json')
    const original = JSON.stringify({ accessToken: 'old', refreshToken: 'refresh', expiresAt: '2035-01-01T00:00:00Z' })
    await writeFile(oauthFile, original)
    for (const raw of malformedSecrets) {
      const store = new FileClaudeCodeAuthStore({ oauthFile, refresh: async () => raw })
      await expect(store.resolveAccess({ forceRefresh: true })).rejects.toMatchObject({ code: 'auth_invalid' })
      expect(await readFile(oauthFile, 'utf8')).toBe(original)
      expect(await readdir(dir)).toEqual(['oauth.json'])
    }
    const store = new FileClaudeCodeAuthStore({ oauthFile, refresh: async () => ({ accessToken: 'new', refreshToken: 'refresh' }) })
    expect((await store.resolveAccess({ forceRefresh: true })).accessToken).toBe('new')
    expect(JSON.parse(await readFile(oauthFile, 'utf8')).accessToken).toBe('new')
    expect((await stat(oauthFile)).mode & 0o777).toBe(0o600)
    expect(await readdir(dir)).toEqual(['oauth.json'])
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('OAuth token response types reject coercion and malformed nested account fields', async () => {
  for (const raw of [
    [], {}, { access_token: 42 }, { access_token: 'token', refresh_token: null },
    { access_token: 'token', expires_in: '' }, { access_token: 'token', expires_in: '3600' },
    { access_token: 'token', expires_in: false }, { access_token: 'token', expires_in: -1 },
    { access_token: 'token', expires_in: 1.5 }, { access_token: 'token', expires_in: 1e20 },
    { access_token: 'token', scope: [] }, { access_token: 'token', scope: '\t' },
    { access_token: 'token', account: [] },
    { access_token: 'token', account: { email_address: 42 } },
    { access_token: 'token', account: { subscription_type: false } },
  ]) {
    await expect(runClaudeCodeLogin({ fetch: fakeFetch(raw), promptForCode: async () => 'code' }))
      .rejects.toMatchObject({ code: 'auth_invalid' })
  }
  const renewed = await refreshClaudeCodeSecret({
    accessToken: 'old', refreshToken: 'refresh', expiresAt: '2020-01-01T00:00:00Z', scopes: ['prior'],
  }, { fetch: fakeFetch({ access_token: 'new', scope: '' }) })
  expect(renewed).toMatchObject({ accessToken: 'new', refreshToken: 'refresh', expiresAt: null, scopes: [] })
})

test('pasted authorization codes reject extra fragments and cancellation stops waiting for input', async () => {
  for (const text of ['', '#state', 'code#', 'code#state#extra', 'code with whitespace']) {
    await expect(runClaudeCodeLogin({ promptForCode: async () => text, fetch: fakeFetch({ access_token: 'token' }) }))
      .rejects.toBeInstanceOf(ClaudeCodeAuthError)
  }
  const controller = new AbortController()
  const login = runClaudeCodeLogin({
    signal: controller.signal, fetch: fakeFetch({ access_token: 'must-not-be-used' }),
    promptForCode: () => new Promise<string>(() => {}),
  })
  controller.abort()
  await expect(login).rejects.toThrow('Aborted')
})
