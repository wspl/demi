import { expect, test } from 'bun:test'
import { Buffer } from 'node:buffer'
import { runGrokDeviceLogin, type GrokDeviceLoginPending } from '../device-login'

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
}

function jwt(payload: Record<string, unknown>): string {
  const header = Buffer.from(JSON.stringify({ alg: 'none', typ: 'JWT' })).toString('base64url')
  const body = Buffer.from(JSON.stringify(payload)).toString('base64url')
  return `${header}.${body}.sig`
}

test('runGrokDeviceLogin drives the official device-flow contract and assembles a vendor-shaped entry', async () => {
  const calls: Array<{ url: string; body: string; headers: Headers }> = []
  let polls = 0
  const idToken = jwt({ sub: 'user_1', email: 'g@example.com' })
  const fakeFetch = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input)
    const headers = new Headers(init?.headers)
    calls.push({ url, body: String(init?.body ?? ''), headers })
    if (url.endsWith('/oauth2/device/code')) {
      return jsonResponse(200, {
        device_code: 'dev_code_1',
        user_code: 'GROK-1234',
        verification_uri: 'https://auth.x.ai/activate',
        verification_uri_complete: 'https://auth.x.ai/activate?user_code=GROK-1234',
        interval: 0,
        expires_in: 600,
      })
    }
    if (url.endsWith('/oauth2/token')) {
      polls += 1
      if (polls === 1) return jsonResponse(400, { error: 'authorization_pending' })
      return jsonResponse(200, { access_token: 'at_1', refresh_token: 'rt_1', expires_in: 3600, id_token: idToken })
    }
    if (url.endsWith('/user')) {
      return jsonResponse(200, { userId: 'user_1', firstName: 'G', email: 'g@example.com' })
    }
    throw new Error(`unexpected fetch: ${url}`)
  }) as typeof fetch

  const pendings: GrokDeviceLoginPending[] = []
  const { entryKey, entry } = await runGrokDeviceLogin({ fetch: fakeFetch, onPending: (p) => pendings.push(p) })

  expect(pendings).toHaveLength(1)
  expect(pendings[0]!.verificationUrl).toBe('https://auth.x.ai/activate?user_code=GROK-1234')
  expect(pendings[0]!.userCode).toBe('GROK-1234')

  expect(entryKey).toBe('https://auth.x.ai::b1a00492-073a-47ea-816f-4c329264a828')
  expect(entry.key).toBe('at_1')
  expect(entry.refresh_token).toBe('rt_1')
  expect(entry.auth_mode).toBe('oidc')
  expect(entry.email).toBe('g@example.com')
  expect(entry.user_id).toBe('user_1')
  expect(entry.first_name).toBe('G')
  expect(entry.oidc_issuer).toBe('https://auth.x.ai')

  const deviceCall = calls.find((c) => c.url.endsWith('/oauth2/device/code'))
  const deviceParams = new URLSearchParams(deviceCall!.body)
  expect(deviceParams.get('client_id')).toBe('b1a00492-073a-47ea-816f-4c329264a828')
  expect(deviceParams.get('referrer')).toBe('grok-build')
  expect(deviceParams.get('scope')).toBe(
    'openid profile email offline_access grok-cli:access api:access conversations:read conversations:write workspaces:read workspaces:write',
  )
  expect(deviceCall!.headers.get('x-grok-client-surface')).toBe('ui')
  expect(deviceCall!.headers.get('x-grok-client-version')).toBeTruthy()

  const tokenCall = calls.filter((c) => c.url.endsWith('/oauth2/token'))
  expect(tokenCall).toHaveLength(2)
  expect(tokenCall[0]!.body).toContain('grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code')
  expect(tokenCall[0]!.headers.get('x-grok-client-surface')).toBe('ui')
  expect(calls.some((c) => c.url.endsWith('/oauth2/userinfo'))).toBe(false)
  expect(calls.some((c) => c.url.endsWith('/user'))).toBe(true)
})

test('runGrokDeviceLogin seeds team principal from the access token', async () => {
  const access = jwt({
    sub: 'user-42',
    principal_type: 'Team',
    principal_id: 'team-123',
  })
  const fakeFetch = (async (input: string | URL | Request) => {
    const url = String(input)
    if (url.endsWith('/oauth2/device/code')) {
      return jsonResponse(200, {
        device_code: 'd',
        user_code: 'TEAM-1',
        verification_uri: 'https://auth.x.ai/activate',
        interval: 0,
        expires_in: 600,
      })
    }
    if (url.endsWith('/oauth2/token')) {
      return jsonResponse(200, { access_token: access, refresh_token: 'rt', expires_in: 3600 })
    }
    if (url.endsWith('/user')) return jsonResponse(404, {})
    throw new Error(`unexpected fetch: ${url}`)
  }) as typeof fetch

  const { entry } = await runGrokDeviceLogin({ fetch: fakeFetch })
  expect(entry.user_id).toBe('team-123')
  expect(entry.principal_type).toBe('Team')
  expect(entry.principal_id).toBe('team-123')
  expect(entry.team_id).toBe('team-123')
  expect(entry.email).toBeUndefined()
})

test('runGrokDeviceLogin fails fast on terminal oauth errors', async () => {
  const fakeFetch = (async (input: string | URL | Request, _init?: RequestInit) => {
    const url = String(input)
    if (url.endsWith('/oauth2/device/code')) {
      return jsonResponse(200, { device_code: 'd', user_code: 'u', verification_uri: 'https://auth.x.ai/activate', interval: 0, expires_in: 600 })
    }
    return jsonResponse(400, { error: 'access_denied' })
  }) as typeof fetch
  await expect(runGrokDeviceLogin({ fetch: fakeFetch })).rejects.toThrow('access_denied')
})
