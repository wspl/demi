import { expect, test } from 'bun:test'
import { providerRuntime } from '@demicodes/provider'
import type { GrokAuthStore, GrokResolvedAuth } from '../auth'
import { createGrokBuildProvider } from '../provider'
import { mapGrokQuotaProbe, observeGrokRateLimitHeaders } from '../quota'

test('mapGrokQuotaProbe maps billing + subscription tier', () => {
  const snap = mapGrokQuotaProbe(
    { subscriptionTier: 'XPremiumPlus', email: 'a@b.com', hasGrokCodeAccess: true },
    {
      config: {
        monthlyLimit: { val: 20000 },
        used: { val: 5000 },
        onDemandCap: { val: 0 },
        billingPeriodEnd: '2026-08-01T00:00:00+00:00',
      },
    },
    { email: 'a@b.com' },
  )
  expect(snap.plan?.id).toBe('XPremiumPlus')
  expect(snap.accountLabel).toBe('a@b.com')
  expect(snap.windows[0]).toMatchObject({
    id: 'monthly',
    usedPercent: 25,
    used: 5000,
    limit: 20000,
    unit: 'credits',
  })
})

test('observeGrokRateLimitHeaders maps short windows', () => {
  const headers = new Headers({
    'x-ratelimit-limit-requests': '120',
    'x-ratelimit-remaining-requests': '100',
    'x-ratelimit-limit-tokens': '5000',
    'x-ratelimit-remaining-tokens': '4000',
  })
  const observed = observeGrokRateLimitHeaders(headers)
  expect(observed?.windows.find((w) => w.id === 'rpm')).toMatchObject({ used: 20, limit: 120 })
  expect(observed?.windows.find((w) => w.id === 'tpm')).toMatchObject({ used: 1000, limit: 5000 })
})

test('Grok inference observes ratelimit headers into provider.quota.latest', async () => {
  const auth: GrokResolvedAuth = {
    accessToken: 'tok',
    refreshToken: null,
    expiresAt: null,
    email: 'a@b.com',
    issuer: null,
    clientId: null,
    entryKey: 'k',
    authFile: '/tmp/auth.json',
  }
  const store: GrokAuthStore = {
    status: async () => ({ status: 'authenticated', accountLabel: 'a@b.com' }),
    resolveAuth: async () => auth,
  }
  const provider = createGrokBuildProvider({
    authStore: store,
    fetch: async () =>
      new Response('data: {"choices":[{"delta":{"content":"hi"}}]}\n\ndata: [DONE]\n\n', {
        status: 200,
        headers: {
          'content-type': 'text/event-stream',
          'x-ratelimit-limit-requests': '100',
          'x-ratelimit-remaining-requests': '90',
        },
      }),
  })
  expect(provider.quota?.latest()).toBeNull()
  const runtime = await providerRuntime(provider, {
    providerId: 'grok-build',
    model: {
      providerId: 'grok-build',
      model: {
        id: 'grok-code-fast-1',
        name: 'Grok',
        contextWindow: 100_000,
        inputLimit: null,
        thinking: [],
        acceptedExtensions: [],
      },
      thinking: null,
    },
  })
  for await (const _ of runtime.run({
    requestId: 'r1',
    turnId: 't1',
    sessionId: 's1',
    modelId: 'grok-code-fast-1',
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
  expect(provider.quota?.latest()?.windows.find((w) => w.id === 'rpm')?.used).toBe(10)
})
