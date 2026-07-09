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

test('createCodexQuota probe uses response headers and cancels body', async () => {
  let seenUrl = ''
  const quota = createCodexQuota({
    authStore: staticStore(),
    fetch: async (input) => {
      seenUrl = String(input)
      return new Response('data: skip\n\n', {
        status: 200,
        headers: {
          'content-type': 'text/event-stream',
          'x-codex-primary-used-percent': '22',
          'x-codex-primary-window-minutes': '300',
          'x-codex-primary-reset-at': '1700000000',
        },
      })
    },
  })
  const snap = await quota.probe()
  expect(seenUrl).toContain('responses')
  expect(snap.windows[0]?.id).toBe('primary')
  expect(snap.windows[0]?.usedPercent).toBe(22)
  expect(snap.accountLabel).toBe('u@example.com')
  expect(quota.capability()).toMatchObject({ probeCost: 'minimal_request', canObserve: true })
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
