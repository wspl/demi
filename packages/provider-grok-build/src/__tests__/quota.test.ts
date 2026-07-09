import { expect, test } from 'bun:test'
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
