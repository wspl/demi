import { expect, test } from 'bun:test'
import { createClaudeCodeQuota, mapClaudeUsagePayload, observeClaudeRateLimitHeaders } from '../quota'

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
    { accessToken: 'tok', subscriptionType: 'max', rateLimitTier: 'default_claude_max_20x' },
  )
  expect(snap.plan?.id).toBe('max')
  expect(snap.windows.find((w) => w.id === 'five_hour')?.usedPercent).toBe(4)
  expect(snap.windows.find((w) => w.id === 'seven_day')?.usedPercent).toBe(70)
  expect(snap.windows.some((w) => w.id.includes('weekly_scoped'))).toBe(true)
})

test('createClaudeCodeQuota probes with mock fetch', async () => {
  const quota = createClaudeCodeQuota({
    resolveAccess: async () => ({ accessToken: 'tok', subscriptionType: 'pro' }),
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
