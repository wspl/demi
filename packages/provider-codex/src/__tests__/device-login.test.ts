import { expect, test } from 'bun:test'
import { Buffer } from 'node:buffer'
import { runCodexDeviceLogin, type CodexDeviceLoginPending } from '../device-login'

function jwt(payload: Record<string, unknown>): string {
  const header = Buffer.from(JSON.stringify({ alg: 'none', typ: 'JWT' })).toString('base64url')
  const body = Buffer.from(JSON.stringify(payload)).toString('base64url')
  return `${header}.${body}.sig`
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
}

const ACCESS_TOKEN = jwt({ email: 'dev@example.com', 'https://api.openai.com/auth': { chatgpt_account_id: 'acc_device' } })
const ID_TOKEN = jwt({ email: 'dev@example.com' })

function scriptedFetch(calls: Array<{ url: string; body: string }>, options: { pendingPolls?: number } = {}): typeof fetch {
  let polls = 0
  return (async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input)
    calls.push({ url, body: String(init?.body ?? '') })
    if (url.endsWith('/deviceauth/usercode')) {
      return jsonResponse(200, { device_auth_id: 'dev_auth_1', user_code: 'WXYZ-9876', interval: '0' })
    }
    if (url.endsWith('/deviceauth/token')) {
      polls += 1
      if (polls <= (options.pendingPolls ?? 1)) return jsonResponse(403, {})
      return jsonResponse(200, { authorization_code: 'authz_1', code_challenge: 'challenge', code_verifier: 'verifier_1' })
    }
    if (url.endsWith('/oauth/token')) {
      return jsonResponse(200, { id_token: ID_TOKEN, access_token: ACCESS_TOKEN, refresh_token: 'refresh_1' })
    }
    throw new Error(`unexpected fetch: ${url}`)
  }) as typeof fetch
}

test('runCodexDeviceLogin surfaces pending material and assembles vendor-shaped auth', async () => {
  const calls: Array<{ url: string; body: string }> = []
  const pendings: CodexDeviceLoginPending[] = []

  const auth = await runCodexDeviceLogin({ fetch: scriptedFetch(calls), onPending: (p) => pendings.push(p) })

  expect(pendings).toHaveLength(1)
  expect(pendings[0]!.verificationUrl).toBe('https://auth.openai.com/codex/device')
  expect(pendings[0]!.userCode).toBe('WXYZ-9876')
  expect(Date.parse(pendings[0]!.expiresAt)).toBeGreaterThan(Date.now())

  expect(auth.auth_mode).toBe('chatgpt')
  expect(auth.tokens?.access_token).toBe(ACCESS_TOKEN)
  expect(auth.tokens?.refresh_token).toBe('refresh_1')
  expect(auth.tokens?.account_id).toBe('acc_device')

  const exchange = calls.find((c) => c.url.endsWith('/oauth/token'))
  expect(exchange).toBeDefined()
  expect(exchange!.body).toContain('grant_type=authorization_code')
  expect(exchange!.body).toContain('code=authz_1')
  expect(exchange!.body).toContain('code_verifier=verifier_1')
  expect(exchange!.body).toContain(encodeURIComponent('https://auth.openai.com/deviceauth/callback'))

  const poll = calls.filter((c) => c.url.endsWith('/deviceauth/token'))
  expect(poll).toHaveLength(2)
  expect(JSON.parse(poll[0]!.body)).toEqual({ device_auth_id: 'dev_auth_1', user_code: 'WXYZ-9876' })
})

test('runCodexDeviceLogin reports unsupported servers distinctly', async () => {
  const fakeFetch = (async (_input: string | URL | Request, _init?: RequestInit) => jsonResponse(404, {})) as typeof fetch
  await expect(runCodexDeviceLogin({ fetch: fakeFetch })).rejects.toThrow('Device-code login is not enabled')
})
