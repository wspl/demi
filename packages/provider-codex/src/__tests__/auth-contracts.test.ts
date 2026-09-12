import { expect, test } from 'bun:test'
import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { ProviderDataError } from '@demicodes/provider'
import { errorCode } from '@demicodes/utils'
import { FileCodexAuthStore, parseChatGptClaims, parseCodexAuthJson } from '../auth'
import { codexDeviceCodeSchema } from '../auth-schemas'
import { createCodexCredentials, openCodexCredentialPool } from '../credentials'
import { runCodexDeviceLogin } from '../device-login'

function jwt(payload: unknown): string {
  return `header.${Buffer.from(JSON.stringify(payload)).toString('base64url')}.signature`
}

const validAuth = {
  auth_mode: 'chatgpt',
  tokens: { access_token: 'opaque-token', refresh_token: 'refresh', account_id: 'account' },
  last_refresh: '2026-09-12T00:00:00.000Z',
}

const malformedAuth = [
  null, [], 42, { auth_mode: 'unknown' },
  { OPENAI_API_KEY: 42 }, { OPENAI_API_KEY: '   ' },
  { tokens: [] }, { tokens: { access_token: false } },
  { tokens: { id_token: [] } }, { tokens: { id_token: { email: 42 } } },
  { tokens: { refresh_token: ' ' } }, { tokens: { account_id: 42 } },
  { last_refresh: 'not-a-date' }, { last_refresh: 0 },
  { agent_identity: { authorization: 'Bearer opaque', account_id: 42 } },
]

test('auth files and credential imports reject malformed structure without writing entries', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'codex-auth-contract-'))
  try {
    const store = new FileCodexAuthStore({ codexHome: dir })
    const pool = openCodexCredentialPool({ stateDir: join(dir, 'pool') })
    const credentials = createCodexCredentials(pool, store, { codexHome: dir })
    for (const value of malformedAuth) {
      const text = JSON.stringify(value)
      expect(() => parseCodexAuthJson(text)).toThrow()
      await writeFile(join(dir, 'auth.json'), text)
      await expect(store.resolveAuth()).rejects.toMatchObject({ code: 'auth_invalid' })
      await expect(credentials.add!({ authJsonText: text })).rejects.toMatchObject({ code: 'auth_invalid' })
    }
    await expect(credentials.add!({ auth: {} })).rejects.toMatchObject({ code: 'auth_missing' })
    await expect(credentials.add!({ auth: validAuth, authJsonText: '{}' })).rejects.toThrow()
    expect(await pool.list()).toEqual([])
    expect(() => parseCodexAuthJson('SYNTHETIC_SECRET_BAD_JSON')).toThrow('invalid JSON')
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('vendor import distinguishes absent files from IO failures', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'codex-import-errors-'))
  try {
    const store = new FileCodexAuthStore({ codexHome: dir })
    const credentials = createCodexCredentials(openCodexCredentialPool({ stateDir: dir }), store, { codexHome: dir })
    await expect(credentials.importDefault!()).rejects.toMatchObject({ code: 'auth_missing' })
    await mkdir(join(dir, 'auth.json'))
    try {
      await credentials.importDefault!()
      throw new Error('Expected directory read failure')
    } catch (error) {
      expect(errorCode(error)).toBe('EISDIR')
    }
    expect((await store.status()).status).toBe('error')
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('bad refresh responses and claims cannot replace credentials and release the file lock', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'codex-refresh-contract-'))
  const text = JSON.stringify(validAuth)
  try {
    await writeFile(join(dir, 'auth.json'), text)
    for (const response of [
      null, {}, { access_token: '' }, { access_token: 42 },
      { access_token: 'new', refresh_token: false },
      { access_token: jwt({ exp: 'tomorrow' }) },
      { access_token: jwt({ 'https://api.openai.com/auth': [] }) },
    ]) {
      const store = new FileCodexAuthStore({ codexHome: dir, refresh: async () => response })
      await expect(store.resolveAuth({ forceRefresh: true })).rejects.toBeInstanceOf(ProviderDataError)
      expect(await readFile(join(dir, 'auth.json'), 'utf8')).toBe(text)
      expect(await readdir(dir)).toEqual(['auth.json'])
    }
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('JWT claims reject wrong shapes while opaque tokens have no metadata', () => {
  for (const claims of [
    [], null, { exp: -1 }, { exp: 1.5 }, { exp: 8_640_000_000_001 },
    { email: [] }, { 'https://api.openai.com/auth': { chatgpt_account_id: 42 } },
    { 'https://api.openai.com/auth': { chatgpt_account_is_fedramp: 'false' } },
    { 'https://api.openai.com/profile': { email: false } },
  ]) {
    expect(() => parseChatGptClaims(jwt(claims))).toThrow(ProviderDataError)
  }
  expect(parseChatGptClaims('opaque-token')).toEqual({ accountId: null, email: null, isFedrampAccount: false })
})

test('device polling intervals accept supported units and reject coercion accidents before polling', async () => {
  for (const interval of [null, false, '', ' ', 'oops', -1, 1.5, 901, [], {}]) {
    let calls = 0
    const fetchImpl: typeof fetch = Object.assign(async () => {
      calls += 1
      return Response.json({ device_auth_id: 'device', user_code: 'ABCD', interval })
    }, { preconnect: () => {} })
    await expect(runCodexDeviceLogin({ fetch: fetchImpl })).rejects.toBeInstanceOf(ProviderDataError)
    expect(calls).toBe(1)
  }
  expect(codexDeviceCodeSchema.parse({ device_auth_id: 'device', user_code: 'ABCD' }).interval).toBe(5)
  for (const interval of [0, '0', 5, '5']) {
    expect(codexDeviceCodeSchema.parse({ device_auth_id: 'device', usercode: 'ABCD', interval }).interval)
      .toBe(Number(interval))
  }
  expect(() => codexDeviceCodeSchema.parse({ device_auth_id: 'device', user_code: 42, usercode: 'ABCD' })).toThrow()
  expect(() => codexDeviceCodeSchema.parse({ device_auth_id: 'device', user_code: 'ABCD', usercode: 'EFGH' })).toThrow()
})

test('device polling cancels its interval timer promptly', async () => {
  const controller = new AbortController()
  let calls = 0
  let timer: ReturnType<typeof setTimeout> | undefined
  const fetchImpl: typeof fetch = Object.assign(async () => {
    calls += 1
    if (calls === 1) {
      return Response.json({ device_auth_id: 'device', user_code: 'ABCD', interval: 900 })
    }
    timer = setTimeout(() => controller.abort(), 1)
    return Response.json({}, { status: 403 })
  }, { preconnect: () => {} })
  try {
    await expect(runCodexDeviceLogin({ fetch: fetchImpl, signal: controller.signal })).rejects.toThrow()
    expect(calls).toBe(2)
  } finally {
    clearTimeout(timer)
  }
})

test('device authorization and token exchange reject malformed fields before returning auth', async () => {
  const authorization = { authorization_code: 'code', code_verifier: 'verifier' }
  const tokens = {
    access_token: jwt({ 'https://api.openai.com/auth': { chatgpt_account_id: 'account' } }),
    id_token: 'opaque-id', refresh_token: 'refresh',
  }
  for (const [grant, exchange] of [
    [{ authorization_code: 42, code_verifier: 'verifier' }, tokens],
    [{ authorization_code: 'code' }, tokens],
    [authorization, { ...tokens, access_token: [] }],
    [authorization, { ...tokens, id_token: 42 }],
    [authorization, { ...tokens, refresh_token: null }],
    [authorization, { ...tokens, access_token: jwt({ exp: 'tomorrow' }) }],
  ]) {
    const responses = [
      { device_auth_id: 'device', user_code: 'ABCD', interval: 0 }, grant, exchange,
    ]
    const fetchImpl: typeof fetch = Object.assign(async () => Response.json(responses.shift()), {
      preconnect: () => {},
    })
    await expect(runCodexDeviceLogin({ fetch: fetchImpl })).rejects.toBeInstanceOf(ProviderDataError)
  }
})
