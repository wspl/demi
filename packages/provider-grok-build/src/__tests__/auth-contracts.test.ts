import { expect, test } from 'bun:test'
import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { FileGrokAuthStore, decodeGrokJwtPayload, parseGrokAuthJson } from '../auth'
import { createGrokBuildCredentials, openGrokCredentialPool, PoolAwareGrokAuthStore } from '../credentials'
import { runGrokDeviceLogin } from '../device-login'

const entryKey = 'https://auth.x.ai::client'
const entry = {
  key: 'secret-access', refresh_token: 'secret-refresh', oidc_client_id: 'client',
  expires_at: '2020-01-01T00:00:00.000Z',
}
const jwt = (claims: unknown) => `header.${Buffer.from(JSON.stringify(claims)).toString('base64url')}.sig`
const response = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status })
const device = {
  device_code: 'private-code', user_code: 'USER-1',
  verification_uri: 'https://auth.x.ai/activate', expires_in: 600, interval: 0,
}

test('Grok file and all import routes reject corrupt entries before writing any credentials', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-grok-contract-'))
  try {
    const pool = openGrokCredentialPool({ stateDir: dir })
    const credentials = createGrokBuildCredentials(pool, new PoolAwareGrokAuthStore(pool, { grokHome: dir }))
    for (const bad of [
      null, [], {}, { key: 12 }, { ...entry, refresh_token: null },
      { ...entry, expires_at: 'yesterday' }, { ...entry, oidc_issuer: 8 },
      { ...entry, email: {} }, { ...entry, principal_id: [] },
      { ...entry, key: jwt({ exp: 'secret-value' }) },
    ]) {
      const text = JSON.stringify({ valid: { key: 'valid' }, [entryKey]: bad })
      expect(() => parseGrokAuthJson(text)).toThrow()
      await expect(credentials.add!({ authJsonText: text })).rejects.toThrow()
      await expect(credentials.add!({ entryKey, entry: bad })).rejects.toThrow()
      expect(await pool.list()).toEqual([])
    }
    await expect(credentials.add!({ authJsonText: '{}', authFile: 'unused' })).rejects.toThrow()
    expect(() => parseGrokAuthJson('{"key":"secret-broken')).toThrow('not valid JSON')
    await writeFile(join(dir, 'auth.json'), JSON.stringify({ [entryKey]: { ...entry, email: [] } }))
    const state = await new FileGrokAuthStore({ grokHome: dir }).status()
    expect(state.status).toBe('error')
    expect(JSON.stringify(state)).not.toContain('secret-access')
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('Grok missing auth differs from invalid IO and malformed sibling entries', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-grok-missing-contract-'))
  try {
    const pool = openGrokCredentialPool({ stateDir: dir })
    const store = new FileGrokAuthStore({ grokHome: dir })
    const credentials = createGrokBuildCredentials(pool, store, { grokHome: dir })
    expect((await store.status()).status).toBe('unauthenticated')
    await expect(credentials.importDefault!()).rejects.toMatchObject({ code: 'auth_missing' })
    await mkdir(join(dir, 'auth.json'))
    expect((await store.status()).status).toBe('error')
    await expect(credentials.importDefault!()).rejects.toMatchObject({ code: 'EISDIR' })
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('Grok refresh rejects invalid responses without replacing files and always releases locks', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-grok-refresh-contract-'))
  const authFile = join(dir, 'auth.json')
  const original = JSON.stringify({ [entryKey]: entry })
  try {
    for (const bad of [
      [], {}, { access_token: 1 }, { access_token: 'new', expires_in: '3600' },
      { access_token: 'new', expires_in: -1 }, { access_token: 'new', expires_in: Infinity },
      { access_token: 'new', expires_in: 1e20 }, { access_token: 'new', refresh_token: false },
      { access_token: jwt({ exp: [] }) },
    ]) {
      await writeFile(authFile, original)
      const store = new FileGrokAuthStore({ authFile, refresh: async () => bad })
      await expect(store.resolveAuth()).rejects.toThrow()
      expect(await readFile(authFile, 'utf8')).toBe(original)
      expect(await readdir(dir)).toEqual(['auth.json'])
    }
    const store = new FileGrokAuthStore({ authFile, refresh: async () => ({ access_token: 'new' }) })
    expect((await store.resolveAuth()).expiresAt).toBeNull()
    expect(JSON.parse(await readFile(authFile, 'utf8'))[entryKey].expires_at).toBeUndefined()
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('shared JWT decoding distinguishes opaque tokens from malformed claims and encodings', () => {
  expect(decodeGrokJwtPayload('opaque')).toBeNull()
  expect(decodeGrokJwtPayload(jwt({ sub: 'u', exp: 0, future: {} })))
    .toMatchObject({ sub: 'u', exp: 0 })
  for (const bad of [
    jwt([]), jwt(null), jwt({ exp: '123' }), jwt({ exp: -1 }), jwt({ exp: 1e20 }),
    jwt({ principal_id: {} }), 'h.*.s', 'h.a.s', 'h._w.s', 'h..s',
  ]) {
    expect(() => decodeGrokJwtPayload(bad)).toThrow()
  }
})

test('Grok device authorization validates optional fields and does not extend expiration', async () => {
  for (const bad of [
    { ...device, interval: null }, { ...device, interval: '5' }, { ...device, interval: -1 },
    { ...device, interval: 0.5 }, { ...device, expires_in: 0 }, { ...device, expires_in: '600' },
    { ...device, expires_in: 1e20 }, { ...device, user_code: 'code with spaces' },
    { ...device, verification_uri_complete: [] }, { ...device, verification_uri: 'file:///tmp/login' },
  ]) {
    let calls = 0
    await expect(runGrokDeviceLogin({ fetch: (async (_input: string | URL | Request) => {
      calls += 1
      return response(bad)
    }) as typeof fetch })).rejects.toThrow()
    expect(calls).toBe(1)
  }
  const controller = new AbortController()
  const before = Date.now()
  let calls = 0
  await expect(runGrokDeviceLogin({
    signal: controller.signal,
    fetch: (async (_input: string | URL | Request) => {
      calls += 1
      return response({ ...device, expires_in: 2, interval: 600 })
    }) as typeof fetch,
    onPending: (pending) => {
      expect(Date.parse(pending.expiresAt) - before).toBeLessThan(3000)
      queueMicrotask(() => controller.abort(new Error('stop polling')))
    },
  })).rejects.toThrow('stop polling')
  expect(Date.now() - before).toBeLessThan(500)
  expect(calls).toBe(1)
})

test('Grok token and successful user profile responses are validated during device login', async () => {
  for (const stage of ['token', 'profile']) {
    await expect(runGrokDeviceLogin({ fetch: (async (input) => {
      const url = String(input)
      if (url.endsWith('/oauth2/device/code')) {
        return response(device)
      }
      if (url.endsWith('/oauth2/token')) {
        return response(stage === 'token'
          ? { access_token: 'private', expires_in: '3600' }
          : { access_token: 'private' })
      }
      return response({ userId: 'user', email: {} })
    }) as typeof fetch })).rejects.toThrow(stage === 'token' ? 'expires_in' : 'email')
  }
})
