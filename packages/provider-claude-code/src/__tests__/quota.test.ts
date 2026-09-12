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
    {
      accessToken: 'tok',
      source: 'static' as const,
      subscriptionType: 'max',
      rateLimitTier: 'default_claude_max_20x'
    },
  )
  expect(snap.plan?.id).toBe('max')
  expect(snap.windows.find((w) => w.id === 'five_hour')?.usedPercent).toBe(4)
  expect(snap.windows.find((w) => w.id === 'seven_day')?.usedPercent).toBe(70)
  expect(snap.windows.some((w) => w.id.includes('weekly_scoped'))).toBe(true)
})

test('createClaudeCodeQuota probes with mock fetch', async () => {
  const quota = createClaudeCodeQuota({
    resolveAccess: async () => ({
      accessToken: 'tok',
      source: 'static' as const,
      subscriptionType: 'pro'
    }),
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
  expect(observed?.windows.find((w) => w.id === 'five_hour')?.usedPercent)
    .toBe(33)
  expect(observed?.windows.find((w) => w.id === 'seven_day')?.usedPercent)
    .toBe(50)
})

test.each([false, true])(
  'Claude stream quota respects account invalidation: %s',
  async (
    invalidate
  ) => {
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
    const factory: ClaudeTransportFactory = { start: async () => {
      if (invalidate)
        quota.clearLatest!()
      return transport
    } }
    const quota = createClaudeCodeQuota({ providerId: 'claude-code' })
    const { ClaudeCodeProvider } = await import('../provider')
    const runtime = new ClaudeCodeProvider({ transportFactory: factory, quota })
    const events = []
    for await (const event of runtime.run({
      requestId: 'req-1',
      turnId: 'turn-1',
      sessionId: 'sess-1',
      outputLimit: null,
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
    if (invalidate) expect(quota.latest()).toBeNull()
    else {
      expect(quota.latest()?.source).toBe('observation')
      expect(
        quota.latest()?.windows.find((w) => w.id === 'five_hour')?.usedPercent
      ).toBe(18)
    }

    // Public shell exposes quota.observeResponse.
    const shell = createClaudeCodeProvider()
    expect(shell.quota).toBeDefined()
    shell.quota!.observeResponse?.({
      body: {
        rate_limits: {
          seven_day: { utilization: 9, resets_at: '2026-07-12T00:00:00.000Z' }
        }
      },
    })
    expect(shell.quota!.latest()?.windows[0]?.usedPercent).toBe(9)
  }
)

class FakeClaudeTransport implements ClaudeTransport, AsyncIterator<unknown> {
  private index = 0
  constructor(private readonly queue: unknown[]) {}
  async writeJson(): Promise<void> {}
  messages(): AsyncIterable<unknown> {
    return { [Symbol.asyncIterator]: () => this }
  }
  async next(): Promise<IteratorResult<unknown>> {
    if (this.index >= this.queue.length)
      return { done: true, value: undefined }
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

test('Claude quota validates known fields while absent and null windows stay unknown', () => {
  expect(mapClaudeUsagePayload({ five_hour: null }).windows).toEqual([])
  expect(mapClaudeUsagePayload({ five_hour: {} }).windows[0])
    .toMatchObject({ usedPercent: null, resetsAt: null })
  expect(mapClaudeUsagePayload({ five_hour: { utilization: 0, resets_at: 0 } }).windows[0])
    .toMatchObject({ usedPercent: 0, resetsAt: '1970-01-01T00:00:00.000Z' })
  for (const value of [
    [], null, { five_hour: [] }, { five_hour: { utilization: '0' } },
    { five_hour: { utilization: NaN } }, { five_hour: { utilization: -1 } },
    { five_hour: { utilization: 2, used_percentage: 'bad' } },
    { seven_day: { resets_at: 'not-a-date' } }, { limits: {} },
    { limits: [{}] }, { limits: [{ kind: 'session', percent: '20' }] },
    { limits: [{ kind: 'session', scope: [] }] },
  ]) {
    expect(() => mapClaudeUsagePayload(value)).toThrow()
  }
  expect(() => observeClaudeStreamBody({ rate_limits: [] })).toThrow()
  expect(() => observeClaudeStreamBody({ message: { rate_limits: false } })).toThrow()
  expect(observeClaudeStreamBody({ type: 'system', future: [] })).toBeNull()
  expect(mapClaudeUsagePayload({ limits: [{ kind: 'session', percent: 12 }] }).windows[0])
    .toMatchObject({ id: 'limit:session', usedPercent: 12 })
})

test('Claude header observation does not guess the overage channel percentage scale', () => {
  const value = observeClaudeRateLimitHeaders(new Headers({
    'anthropic-ratelimit-unified-overage-period-channel-utilization': '0.5',
    'anthropic-ratelimit-unified-status': 'allowed_warning',
  }))
  expect(value?.windows[0]).toMatchObject({ usedPercent: null, severity: 'warning' })
  expect(value?.raw).toMatchObject({ overageUtil: 0.5 })
  expect(() => observeClaudeRateLimitHeaders(new Headers({
    'anthropic-ratelimit-unified-reset': 'invalid',
  }))).toThrow()
})

test('malformed Claude quota observations preserve the previous cache', () => {
  const quota = createClaudeCodeQuota({ resolveAccess: async () => null })
  quota.observeResponse!({ body: { rate_limits: { five_hour: { utilization: 12 } } } })
  expect(() => quota.observeResponse!({ body: { rate_limits: { five_hour: { utilization: '12' } } } }))
    .toThrow()
  expect(quota.latest()?.windows[0]?.usedPercent).toBe(12)
})
