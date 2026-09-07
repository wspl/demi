import { expect, test } from 'bun:test'
import type { CodexAuthStore, CodexResolvedAuth } from '../auth'
import { createCodexProvider } from '../provider'
import { createCodexQuota, mapCodexRateLimitHeaders } from '../quota'
import { FetchCodexResponsesTransport } from '../transport'

const staticAuth: CodexResolvedAuth = {
  kind: 'chatgpt',
  mode: 'chatgpt',
  accessToken: 'tok',
  refreshToken: null,
  accountId: 'acct',
  email: 'u@example.com',
  authFile: '/tmp/auth.json',
  isFedrampAccount: false,
  expiresAt: null,
}

function staticStore(): CodexAuthStore {
  return {
    status: async () => ({ status: 'authenticated', accountLabel: 'u@example.com' }),
    resolveAuth: async () => staticAuth,
  }
}

test('mapCodexRateLimitHeaders maps primary and secondary windows', () => {
  const headers = new Headers({
    'x-codex-primary-used-percent': '35.5',
    'x-codex-primary-window-minutes': '300',
    'x-codex-primary-reset-at': '1700000000',
    'x-codex-secondary-used-percent': '10',
    'x-codex-secondary-window-minutes': '10080',
    'x-codex-secondary-reset-at': '1700500000',
  })
  const mapped = mapCodexRateLimitHeaders(headers)
  expect(mapped?.windows).toHaveLength(2)
  expect(mapped?.windows[0]).toMatchObject({ id: 'primary', usedPercent: 35.5 })
  expect(mapped?.windows[1]).toMatchObject({ id: 'secondary', usedPercent: 10 })
})

test('createCodexQuota probes usage without invoking a model', async () => {
  const quota = createCodexQuota({
    authStore: staticStore(),
    fetch: async (input, init) => {
      expect(String(input)).toBe('https://chatgpt.com/backend-api/wham/usage')
      expect(init?.method).toBe('GET')
      expect(init?.body).toBeUndefined()
      const headers = new Headers(init?.headers)
      expect(headers.get('authorization')).toBe('Bearer tok')
      expect(headers.get('chatgpt-account-id')).toBe('acct')
      return Response.json({
        plan_type: 'pro',
        rate_limit: { primary_window: { used_percent: 22, limit_window_seconds: 604800, reset_at: 1700000000 }, secondary_window: null },
        additional_rate_limits: [{ limit_name: 'Spark', metered_feature: 'codex_spark', rate_limit: {
          primary_window: { used_percent: 10, limit_window_seconds: 18000, reset_at: 1700500000 },
        } }],
      })
    },
  })
  const snap = await quota.probe()
  expect(snap.windows).toHaveLength(2)
  expect(snap.windows[0]).toMatchObject({ id: 'primary', usedPercent: 22, label: 'Primary (10080m)' })
  expect(snap.windows[1]).toMatchObject({ id: 'codex_spark:primary', usedPercent: 10, scope: { kind: 'model', label: 'Spark' } })
  expect(snap.plan?.id).toBe('pro')
  expect(snap.accountLabel).toBe('u@example.com')
  expect(quota.capability()).toMatchObject({ probeCost: 'free', canObserve: true })
})

test('Codex SSE inference observes x-codex headers into provider.quota.latest', async () => {
  const provider = createCodexProvider({
    authStore: staticStore(),
    transport: 'sse',
  })
  // Rebuild runtime with a fetch transport that returns ratelimit headers.
  const transport = new FetchCodexResponsesTransport({
    fetch: (async () =>
      new Response('event: response.completed\ndata: {"type":"response.completed","response":{"id":"r1"}}\n\n', {
        status: 200,
        headers: {
          'content-type': 'text/event-stream',
          'x-codex-primary-used-percent': '41',
          'x-codex-primary-window-minutes': '300',
          'x-codex-primary-reset-at': '1700000000',
        },
      })) as unknown as typeof fetch,
  })
  const { CodexProvider } = await import('../provider')
  const runtime = new CodexProvider({
    authStore: staticStore(),
    transportImpl: transport,
    quota: provider.quota,
  })
  for await (const _ of runtime.run({
    requestId: 'r1',
    turnId: 't1',
    sessionId: 's1',
    modelId: 'gpt-5.4',
    systemPrompt: 'sys',
    cwd: '/tmp',
    items: [{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }],
    tools: [],
    thinking: null,
    cancel: new AbortController().signal,
  })) {
    // drain
  }
  expect(provider.quota?.latest()?.source).toBe('observation')
  expect(provider.quota?.latest()?.windows[0]?.usedPercent).toBe(41)
})

test('Codex inference is not interrupted when quota observation throws', async () => {
  const quota = createCodexQuota({ authStore: staticStore() })
  quota.observeResponse = () => {
    throw new Error('broken observer')
  }
  const transport = new FetchCodexResponsesTransport({
    fetch: (async () =>
      new Response('event: response.completed\ndata: {"type":"response.completed","response":{"id":"r1"}}\n\n', {
        status: 200,
        headers: { 'content-type': 'text/event-stream', 'x-codex-primary-used-percent': '41' },
      })) as unknown as typeof fetch,
  })
  const { CodexProvider } = await import('../provider')
  const runtime = new CodexProvider({ authStore: staticStore(), transportImpl: transport, quota })
  const events = []

  for await (const event of runtime.run({
    requestId: 'r1',
    turnId: 't1',
    sessionId: 's1',
    modelId: 'gpt-5.4',
    systemPrompt: 'sys',
    cwd: '/tmp',
    items: [{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }],
    tools: [],
    thinking: null,
    cancel: new AbortController().signal,
  })) {
    events.push(event)
  }

  expect(events.some((event) => event.type === 'response')).toBe(true)
  expect(events.some((event) => event.type === 'error')).toBe(false)
})

test('Codex usage refreshes once on 401 and preserves cancellation', async () => {
  const refreshes: boolean[] = []
  const signal = new AbortController().signal
  let calls = 0
  const quota = createCodexQuota({
    authStore: {
      ...staticStore(),
      resolveAuth: async (options) => {
        refreshes.push(options?.forceRefresh === true)
        return { ...staticAuth, accessToken: options?.forceRefresh ? 'renewed' : 'tok' }
      },
    },
    fetch: async (_input, init) => {
      expect(init?.signal).toBe(signal)
      calls += 1
      if (calls === 1) return new Response(null, { status: 401 })
      expect(new Headers(init?.headers).get('authorization')).toBe('Bearer renewed')
      return Response.json({ plan_type: 'pro', rate_limit: null })
    },
  })
  expect((await quota.probe({ signal })).windows).toEqual([])
  expect(refreshes).toEqual([false, true])
})

for (const status of [401, 403, 429, 500]) {
  test(`Codex usage reports HTTP ${status} without treating error headers as quota`, async () => {
    let calls = 0
    const quota = createCodexQuota({
      authStore: staticStore(),
      fetch: async () => {
        calls += 1
        return new Response('private error detail', { status, headers: { 'x-codex-primary-used-percent': '100' } })
      },
    })
    await expect(quota.probe()).rejects.toThrow(`Codex usage request failed with HTTP ${status}`)
    expect(calls).toBe(status === 401 ? 2 : 1)
    expect(quota.latest()).toBeNull()
  })
}

test('Codex usage rejects malformed payloads', async () => {
  const quota = createCodexQuota({ authStore: staticStore(), fetch: async () => Response.json({}) })
  await expect(quota.probe()).rejects.toThrow('invalid quota data')
})

test('Codex usage rejects API keys before fetching', async () => {
  const quota = createCodexQuota({
    authStore: { ...staticStore(), resolveAuth: async () => ({ kind: 'apiKey', mode: 'apiKey', apiKey: 'secret', authFile: null }) },
    fetch: async () => { throw new Error('must not fetch') },
  })
  await expect(quota.probe()).rejects.toThrow('requires ChatGPT account authentication')
})

test('Codex usage propagates an aborted signal before auth or fetch', async () => {
  const quota = createCodexQuota({
    authStore: { ...staticStore(), resolveAuth: async () => { throw new Error('must not resolve') } },
    fetch: async () => { throw new Error('must not fetch') },
  })
  await expect(quota.probe({ signal: AbortSignal.abort(new Error('cancelled')) })).rejects.toThrow('cancelled')
})
