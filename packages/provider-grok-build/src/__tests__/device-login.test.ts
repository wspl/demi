import { expect, test } from 'bun:test'
import { runGrokDeviceLogin, type GrokDeviceLoginPending } from '../device-login'

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
}

test('runGrokDeviceLogin drives the RFC 8628 flow and assembles a vendor-shaped entry', async () => {
  const calls: Array<{ url: string; body: string; auth: string }> = []
  let polls = 0
  const fakeFetch = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input)
    const headers = new Headers(init?.headers)
    calls.push({ url, body: String(init?.body ?? ''), auth: headers.get('authorization') ?? '' })
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
      return jsonResponse(200, { access_token: 'at_1', refresh_token: 'rt_1', expires_in: 3600 })
    }
    if (url.endsWith('/oauth2/userinfo')) {
      return jsonResponse(200, { sub: 'user_1', email: 'g@example.com', given_name: 'G' })
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
  expect(entry.oidc_issuer).toBe('https://auth.x.ai')

  const tokenCall = calls.filter((c) => c.url.endsWith('/oauth2/token'))
  expect(tokenCall).toHaveLength(2)
  expect(tokenCall[0]!.body).toContain('grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code')
  const userinfo = calls.find((c) => c.url.endsWith('/oauth2/userinfo'))
  expect(userinfo!.auth).toBe('Bearer at_1')
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
