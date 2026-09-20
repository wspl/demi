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
    status: async () => ({
      status: 'authenticated',
      accountLabel: 'u@example.com'
    }),
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
  expect(mapped?.windows[0]).toMatchObject({ id: 'primary', label: '5-hour', usedPercent: 35.5 })
  expect(mapped?.windows[1]).toMatchObject({ id: 'secondary', label: 'Weekly', usedPercent: 10 })
})

test('the probe reads the account\'s usage status: free, with the plan and the windows named by their length', async () => {
  let seen: { url: string, method?: string, account: string | null } | undefined
  const quota = createCodexQuota({
    authStore: staticStore(),
    fetch: async (input, init) => {
      seen = {
        url: String(input),
        method: init?.method,
        account: new Headers(init?.headers).get('ChatGPT-Account-Id'),
      }
      return Response.json({
        plan_type: 'pro',
        rate_limit: {
          allowed: false,
          limit_reached: true,
          primary_window: { used_percent: 100, limit_window_seconds: 18_000, reset_after_seconds: 60, reset_at: 1_700_000_000 },
          secondary_window: { used_percent: 41, limit_window_seconds: 604_800, reset_after_seconds: 900, reset_at: 1_700_500_000 },
        },
      })
    },
  })
  const snap = await quota.probe()
  expect(seen).toEqual({ url: 'https://chatgpt.com/backend-api/wham/usage', method: 'GET', account: 'acct' })
  expect(snap.plan).toMatchObject({ id: 'pro', label: 'Pro' })
  expect(snap.windows.map(window => [window.id, window.label, window.usedPercent])).toEqual([
    ['primary', '5-hour', 100],
    ['secondary', 'Weekly', 41],
  ])
  expect(snap.windows[0]?.resetsAt).toBe(new Date(1_700_000_000_000).toISOString())
  expect(snap.accountLabel).toBe('u@example.com')
  expect(quota.capability()).toMatchObject({ probeCost: 'free', canObserve: true })
})

test('a usage status the backend refuses, and an API key that has none, fail the probe', async () => {
  const refused = createCodexQuota({
    authStore: staticStore(),
    fetch: async () => new Response('nope', { status: 403 }),
  })
  await expect(refused.probe()).rejects.toThrow('HTTP 403')
  const apiKey = createCodexQuota({
    authStore: {
      status: async () => ({ status: 'authenticated', accountLabel: 'OPENAI_API_KEY' }),
      resolveAuth: async () => ({ kind: 'apiKey', apiKey: 'sk-test' }) as CodexResolvedAuth,
    },
    fetch: async () => { throw new Error('no request expected') },
  })
  await expect(apiKey.probe()).rejects.toThrow('no Codex usage status')
})

test.each([false, true])(
  'Codex SSE quota respects account invalidation: %s',
  async (
    invalidate
  ) => {
    const provider = createCodexProvider({
      authStore: staticStore(),
      transport: 'sse',
    })
    // Rebuild runtime with a fetch transport that returns ratelimit headers.
    const transport = new FetchCodexResponsesTransport({
      fetch: (async () => {
        if (invalidate)
          provider.quota!.clearLatest!()
        return new Response(
          'event: response.completed\ndata: {"type":"response.completed","response":{"id":"r1"}}\n\n',
          {
            status: 200,
            headers: {
              'content-type': 'text/event-stream',
              'x-codex-primary-used-percent': '41',
              'x-codex-primary-window-minutes': '300',
              'x-codex-primary-reset-at': '1700000000',
            },
          }
        )
      }) as unknown as typeof fetch,
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
      outputLimit: null,
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
    if (invalidate) expect(provider.quota?.latest()).toBeNull()
    else {
      expect(provider.quota?.latest()?.source).toBe('observation')
      expect(provider.quota?.latest()?.windows[0]?.usedPercent).toBe(41)
    }
  }
)

test(
  'Codex inference is not interrupted when quota observation throws',
  async () => {
    const quota = createCodexQuota({ authStore: staticStore() })
    quota.observeResponse = () => {
      throw new Error('broken observer')
    }
    const transport = new FetchCodexResponsesTransport({
      fetch: (async () =>
        new Response(
          'event: response.completed\ndata: {"type":"response.completed","response":{"id":"r1"}}\n\n',
          {
            status: 200,
            headers: {
              'content-type': 'text/event-stream',
              'x-codex-primary-used-percent': '41'
            },
          }
        )) as unknown as typeof fetch,
    })
    const { CodexProvider } = await import('../provider')
    const runtime = new CodexProvider({
      authStore: staticStore(),
      transportImpl: transport,
      quota
    })
    const events = []

    for await (const event of runtime.run({
      requestId: 'r1',
      turnId: 't1',
      sessionId: 's1',
      outputLimit: null,
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
  }
)
