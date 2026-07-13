import { expect, test } from 'bun:test'
import { refreshClaudeCodeSecret, runClaudeCodeLogin } from '../login'

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
}

test('runClaudeCodeLogin builds a PKCE authorize URL and exchanges the pasted code', async () => {
  const calls: Array<{ url: string; body: string }> = []
  const fakeFetch = (async (input: string | URL | Request, init?: RequestInit) => {
    calls.push({ url: String(input), body: String(init?.body ?? '') })
    return jsonResponse(200, {
      access_token: 'at_claude',
      refresh_token: 'rt_claude',
      expires_in: 28800,
      scope: 'user:inference user:profile',
      account: { email_address: 'c@example.com', subscription_type: 'max' },
    })
  }) as typeof fetch

  let pendingUrl = ''
  const secret = await runClaudeCodeLogin({
    fetch: fakeFetch,
    onPending: (p) => {
      pendingUrl = p.verificationUrl
      expect(p.requiresCodeInput).toBe(true)
    },
    promptForCode: async () => {
      // The user pastes "code#state" from the callback page; echo the real state back.
      const state = new URL(pendingUrl).searchParams.get('state')!
      return `the-auth-code#${state}`
    },
  })

  const authorize = new URL(pendingUrl)
  expect(authorize.origin + authorize.pathname).toBe('https://claude.ai/oauth/authorize')
  expect(authorize.searchParams.get('client_id')).toBe('9d1c250a-e61b-44d9-88ed-5944d1962f5e')
  expect(authorize.searchParams.get('code_challenge_method')).toBe('S256')
  expect(authorize.searchParams.get('scope')).toBe('org:create_api_key user:profile user:inference')

  expect(secret.accessToken).toBe('at_claude')
  expect(secret.refreshToken).toBe('rt_claude')
  expect(secret.subscriptionType).toBe('max')
  expect(secret.emailAddress).toBe('c@example.com')
  expect(Date.parse(secret.expiresAt!)).toBeGreaterThan(Date.now())

  expect(calls).toHaveLength(1)
  const body = JSON.parse(calls[0]!.body) as Record<string, string>
  expect(calls[0]!.url).toBe('https://console.anthropic.com/v1/oauth/token')
  expect(body.grant_type).toBe('authorization_code')
  expect(body.code).toBe('the-auth-code')
  expect(body.code_verifier).toBeTruthy()
})

test('runClaudeCodeLogin rejects a state mismatch', async () => {
  const fakeFetch = (async (_input: string | URL | Request, _init?: RequestInit) => jsonResponse(200, {})) as typeof fetch
  await expect(
    runClaudeCodeLogin({
      fetch: fakeFetch,
      promptForCode: async () => 'code#wrong-state',
    }),
  ).rejects.toThrow('state mismatch')
})

test('refreshClaudeCodeSecret renews tokens and keeps the refresh token on rotation gaps', async () => {
  const fakeFetch = (async (_input: string | URL | Request, init?: RequestInit) => {
    const body = JSON.parse(String(init?.body ?? '{}')) as Record<string, string>
    expect(body.grant_type).toBe('refresh_token')
    expect(body.refresh_token).toBe('rt_old')
    return jsonResponse(200, { access_token: 'at_new', expires_in: 28800 })
  }) as typeof fetch

  const renewed = await refreshClaudeCodeSecret(
    { accessToken: 'at_old', refreshToken: 'rt_old', subscriptionType: 'max' },
    { fetch: fakeFetch },
  )
  expect(renewed.accessToken).toBe('at_new')
  expect(renewed.refreshToken).toBe('rt_old')
  expect(renewed.subscriptionType).toBe('max')
})
