import { expect, test } from 'bun:test'
import { Buffer } from 'node:buffer'
import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  FileCodexAuthStore,
  parseChatGptClaims,
  parseJwtExpiration,
  redactSecretText,
  resolvedAuthMode,
  StaticCodexAuthStore,
} from '../auth'

test('FileCodexAuthStore resolves ChatGPT auth from official auth.json shape', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-codex-auth-'))
  try {
    const access = jwt({
      exp: 1_900_000_000,
      email: 'access@example.com',
      'https://api.openai.com/auth': {
        chatgpt_account_id: 'account-from-access',
        chatgpt_account_is_fedramp: true,
      },
    })
    const id = jwt({
      email: 'id@example.com',
      'https://api.openai.com/auth': { chatgpt_account_id: 'account-from-id' },
    })
    await writeFile(
      join(dir, 'auth.json'),
      JSON.stringify({
        auth_mode: 'chatgpt',
        tokens: {
          id_token: id,
          access_token: access,
          refresh_token: 'refresh-secret',
          account_id: 'account-from-file',
        },
        last_refresh: '2026-06-18T00:00:00.000Z',
      }),
    )

    const store = new FileCodexAuthStore({ codexHome: dir, now: () => new Date('2026-06-19T00:00:00.000Z') })
    const auth = await store.resolveAuth()

    expect(auth).toMatchObject({
      kind: 'chatgpt',
      mode: 'chatgpt',
      accountId: 'account-from-file',
      email: 'id@example.com',
      isFedrampAccount: true,
    })
    expect(auth.kind === 'chatgpt' ? auth.accessToken : null).toBe(access)
    expect(await store.status()).toEqual({ status: 'authenticated', accountLabel: 'id@example.com' })
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('FileCodexAuthStore refreshes near-expiry ChatGPT tokens and preserves unknown fields', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-codex-refresh-'))
  const now = new Date('2026-06-19T00:00:00.000Z')
  const oldAccess = jwt({
    exp: Math.floor((now.getTime() + 30_000) / 1000),
    'https://api.openai.com/auth': { chatgpt_account_id: 'account-1' },
  })
  const newAccess = jwt({
    exp: Math.floor((now.getTime() + 3_600_000) / 1000),
    'https://api.openai.com/auth': { chatgpt_account_id: 'account-1' },
  })
  try {
    await writeFile(
      join(dir, 'auth.json'),
      `${JSON.stringify({
        auth_mode: 'chatgpt',
        tokens: {
          id_token: jwt({ email: 'before@example.com' }),
          access_token: oldAccess,
          refresh_token: 'refresh-old',
          account_id: 'account-1',
        },
        last_refresh: '2026-06-18T00:00:00.000Z',
        custom_future_field: { keep: true },
      })}\n`,
      { mode: 0o600 },
    )

    const store = new FileCodexAuthStore({
      codexHome: dir,
      now: () => now,
      refresh: async (refreshToken) => {
        expect(refreshToken).toBe('refresh-old')
        return {
          id_token: jwt({ email: 'after@example.com' }),
          access_token: newAccess,
          refresh_token: 'refresh-new',
        }
      },
    })
    const auth = await store.resolveAuth()
    const written = JSON.parse(await readFile(join(dir, 'auth.json'), 'utf8'))
    const mode = (await stat(join(dir, 'auth.json'))).mode & 0o777

    expect(auth.kind === 'chatgpt' ? auth.accessToken : null).toBe(newAccess)
    expect(auth.kind === 'chatgpt' ? auth.email : null).toBe('after@example.com')
    expect(written.custom_future_field).toEqual({ keep: true })
    expect(written.tokens.refresh_token).toBe('refresh-new')
    expect(written.last_refresh).toBe(now.toISOString())
    expect(mode).toBe(0o600)
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('FileCodexAuthStore resolves API key and reports missing auth without secret leakage', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-codex-api-key-'))
  try {
    await writeFile(join(dir, 'auth.json'), JSON.stringify({ auth_mode: 'apiKey', OPENAI_API_KEY: 'sk-secret' }))
    const store = new FileCodexAuthStore({ codexHome: dir })
    expect(await store.resolveAuth()).toEqual({
      kind: 'apiKey',
      mode: 'apiKey',
      apiKey: 'sk-secret',
      authFile: join(dir, 'auth.json'),
    })

    await writeFile(join(dir, 'auth.json'), JSON.stringify({ auth_mode: 'apiKey' }))
    const status = await store.status()
    expect(status.status).toBe('unauthenticated')
    expect(JSON.stringify(status)).not.toContain('sk-secret')
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test('JWT helpers parse ChatGPT account claims conservatively', () => {
  const token = jwt({
    exp: 1_900_000_000,
    email: 'dev@example.com',
    'https://api.openai.com/auth': {
      chatgpt_account_id: 'account-123',
      chatgpt_account_is_fedramp: true,
    },
  })

  expect(parseChatGptClaims(token)).toEqual({
    accountId: 'account-123',
    email: 'dev@example.com',
    isFedrampAccount: true,
  })
  expect(parseJwtExpiration(token)?.toISOString()).toBe('2030-03-17T17:46:40.000Z')
  expect(parseChatGptClaims('not-a-jwt')).toEqual({ accountId: null, email: null, isFedrampAccount: false })
})

test('auth mode resolution and redaction follow official auth.json precedence', async () => {
  expect(resolvedAuthMode({ personal_access_token: 'pat' })).toBe('personalAccessToken')
  expect(resolvedAuthMode({ OPENAI_API_KEY: 'sk' })).toBe('apiKey')
  expect(resolvedAuthMode({ tokens: {} })).toBe('chatgpt')
  expect(redactSecretText('Authorization: Bearer abc.def.ghi access_token: secret')).not.toContain('abc.def.ghi')

  const staticStore = new StaticCodexAuthStore({
    kind: 'apiKey',
    mode: 'apiKey',
    apiKey: 'sk-test',
    authFile: null,
  })
  expect(await staticStore.status()).toEqual({ status: 'authenticated', accountLabel: 'apiKey' })
})

function jwt(payload: Record<string, unknown>): string {
  return [base64Url({ alg: 'none', typ: 'JWT' }), base64Url(payload), 'signature'].join('.')
}

function base64Url(value: Record<string, unknown>): string {
  return Buffer.from(JSON.stringify(value)).toString('base64url')
}
