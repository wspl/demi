import { expect, test } from 'bun:test'
import { createClaudeCodeProvider } from '../provider'
import {
  createClaudeCodeQuota,
  mapClaudeUsagePayload,
  observeClaudeRateLimitHeaders,
  observeClaudeStreamBody,
} from '../quota'
import type { ClaudeTransport, ClaudeTransportFactory } from '../transport'

test('mapClaudeUsagePayload maps five_hour and seven_day', () => {
  const snap = mapClaudeUsagePayload(
    {
      five_hour: { utilization: 4, resets_at: '2026-07-09T09:00:00.000Z' },
      seven_day: { utilization: 70, resets_at: '2026-07-11T08:00:00.000Z' },
      limits: [
        {
          kind: 'weekly_scoped',
          percent: 100,
          severity: 'critical',
          resets_at: '2026-07-11T08:00:00.000Z',
          scope: { model: { display_name: 'Fable' } },
        },
      ],
    },
    { accessToken: 'tok', source: 'static' as const, subscriptionType: 'max', rateLimitTier: 'default_claude_max_20x' },
  )
  expect(snap.plan?.id).toBe('max')
  expect(snap.windows.find((w) => w.id === 'five_hour')?.usedPercent).toBe(4)
  expect(snap.windows.find((w) => w.id === 'seven_day')?.usedPercent).toBe(70)
  expect(snap.windows.some((w) => w.id.includes('weekly_scoped'))).toBe(true)
})

test('createClaudeCodeQuota probes with mock fetch', async () => {
  const quota = createClaudeCodeQuota({
    resolveAccess: async () => ({ accessToken: 'tok', source: 'static' as const, subscriptionType: 'pro' }),
    fetch: async () =>
      new Response(
        JSON.stringify({
          five_hour: { utilization: 12, resets_at: '2026-07-09T10:00:00.000Z' },
          seven_day: { utilization: 40, resets_at: '2026-07-12T00:00:00.000Z' },
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      ),
  })
  const snap = await quota.probe()
  expect(snap.source).toBe('probe')
  expect(snap.plan?.id).toBe('pro')
  expect(snap.windows[0]?.id).toBe('five_hour')
  expect(snap.windows[0]?.usedPercent).toBe(12)
})

test('observeClaudeRateLimitHeaders', () => {
  const headers = new Headers({
    'anthropic-ratelimit-unified-status': 'allowed',
    'anthropic-ratelimit-unified-reset': '1700000000',
    'anthropic-ratelimit-unified-representative-claim': 'five_hour',
  })
  const observed = observeClaudeRateLimitHeaders(headers)
  expect(observed?.windows[0]?.id).toBe('unified')
  expect(observed?.windows[0]?.label).toBe('five_hour')
})

test('observeClaudeStreamBody maps rate_limits on stream messages', () => {
  const observed = observeClaudeStreamBody({
    type: 'result',
    rate_limits: {
      five_hour: { used_percentage: 33, resets_at: '2026-07-09T09:00:00.000Z' },
      seven_day: { utilization: 50, resets_at: '2026-07-11T08:00:00.000Z' },
    },
  })
  expect(observed?.windows.find((w) => w.id === 'five_hour')?.usedPercent).toBe(33)
  expect(observed?.windows.find((w) => w.id === 'seven_day')?.usedPercent).toBe(50)
})

test('Claude inference observes rate_limits from stream-json messages into shared quota', async () => {
  const transport = new FakeClaudeTransport([
    {
      type: 'result',
      subtype: 'success',
      result: 'ok',
      usage: { input_tokens: 1, output_tokens: 1 },
      rate_limits: {
        five_hour: { utilization: 18, resets_at: '2026-07-09T12:00:00.000Z' },
      },
    },
  ])
  const factory: ClaudeTransportFactory = { start: async () => transport }
  const quota = createClaudeCodeQuota({ providerId: 'claude-code' })
  const { ClaudeCodeProvider } = await import('../provider')
  const runtime = new ClaudeCodeProvider({ transportFactory: factory, quota })
  const events = []
  for await (const event of runtime.run({
    requestId: 'req-1',
    turnId: 'turn-1',
    sessionId: 'sess-1',
    modelId: 'claude-sonnet-4-6',
    systemPrompt: 'sys',
    cwd: '/tmp',
    items: [{ type: 'user_message', content: [{ type: 'text', text: 'hi' }] }],
    tools: [],
    thinking: null,
    cancel: new AbortController().signal,
  })) {
    events.push(event)
  }
  expect(events.some((e) => e.type === 'response')).toBe(true)
  expect(quota.latest()?.source).toBe('observation')
  expect(quota.latest()?.windows.find((w) => w.id === 'five_hour')?.usedPercent).toBe(18)

  // Public shell exposes quota.observeResponse.
  const shell = createClaudeCodeProvider()
  expect(shell.quota).toBeDefined()
  shell.quota!.observeResponse?.({
    body: { rate_limits: { seven_day: { utilization: 9, resets_at: '2026-07-12T00:00:00.000Z' } } },
  })
  expect(shell.quota!.latest()?.windows[0]?.usedPercent).toBe(9)
})

class FakeClaudeTransport implements ClaudeTransport, AsyncIterator<unknown> {
  private index = 0
  constructor(private readonly queue: unknown[]) {}
  async writeJson(): Promise<void> {}
  messages(): AsyncIterable<unknown> {
    return { [Symbol.asyncIterator]: () => this }
  }
  async next(): Promise<IteratorResult<unknown>> {
    if (this.index >= this.queue.length) return { done: true, value: undefined }
    const value = this.queue[this.index++]
    return { done: false, value }
  }
  async kill(): Promise<void> {}
  async wait(): Promise<{ exitCode: number | null }> {
    return { exitCode: 0 }
  }
  stderrText(): string {
    return ''
  }
}
