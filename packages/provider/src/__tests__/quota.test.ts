import { expect, test } from 'bun:test'
import {
  clampUsedPercent,
  createProviderQuota,
  ensureQuota,
  ProviderQuotaUnsupportedError,
  severityFromUsedPercent,
  unixSecondsToIso,
  usedPercentFromRatio,
} from '../quota'

test('createProviderQuota probes and caches latest', async () => {
  let probes = 0
  const quota = createProviderQuota({
    providerId: 'demo',
    canProbe: true,
    canObserve: true,
    staleAfterMs: 60_000,
    probe: async () => {
      probes += 1
      return {
        plan: { id: 'pro', label: 'Pro' },
        windows: [{ id: 'monthly', usedPercent: 10, resetsAt: null }],
      }
    },
    observe: ({ headers }) => {
      const used = headers?.get('x-used-percent')
      if (used == null) return null
      return {
        windows: [{ id: 'rpm', usedPercent: Number(used), resetsAt: null }],
      }
    },
  })

  expect(quota.capability()).toMatchObject({ mode: 'supported', canProbe: true, canObserve: true })
  expect(quota.latest()).toBeNull()

  const snap = await quota.probe()
  expect(snap.providerId).toBe('demo')
  expect(snap.source).toBe('probe')
  expect(snap.windows[0]?.usedPercent).toBe(10)
  expect(probes).toBe(1)
  expect(quota.latest()?.source).toBe('probe')

  const headers = new Headers({ 'x-used-percent': '42' })
  const observed = quota.observeResponse?.({ headers })
  expect(observed?.source).toBe('observation')
  expect(observed?.windows.find((window) => window.id === 'rpm')?.usedPercent).toBe(42)
  expect(quota.latest()?.windows.find((window) => window.id === 'rpm')?.usedPercent).toBe(42)

  const ensured = await ensureQuota(quota, { prefer: 'cache', maxStaleMs: 60_000 })
  expect(ensured?.source).toBe('cache')
  expect(probes).toBe(1)

  await ensureQuota(quota, { prefer: 'probe' })
  expect(probes).toBe(2)

  quota.clearLatest?.()
  expect(quota.latest()).toBeNull()
})

test('probe throws when unsupported', async () => {
  const quota = createProviderQuota({
    providerId: 'none',
    canProbe: false,
    probe: async () => ({ windows: [] }),
  })
  await expect(quota.probe()).rejects.toBeInstanceOf(ProviderQuotaUnsupportedError)
})

test('observations merge partial windows without dropping probed plan and quota', async () => {
  const quota = createProviderQuota({
    providerId: 'demo',
    canProbe: true,
    canObserve: true,
    probe: async () => ({
      plan: { id: 'pro' },
      accountLabel: 'person@example.com',
      windows: [
        { id: 'monthly', usedPercent: 25, resetsAt: null },
        { id: 'rpm', usedPercent: 10, resetsAt: null },
      ],
    }),
    observe: () => ({
      windows: [
        { id: 'rpm', usedPercent: 40, resetsAt: null },
        { id: 'tpm', usedPercent: 5, resetsAt: null },
      ],
    }),
  })

  await quota.probe()
  const observed = quota.observeResponse?.({})

  expect(observed?.plan?.id).toBe('pro')
  expect(observed?.accountLabel).toBe('person@example.com')
  expect(observed?.windows.map((window) => [window.id, window.usedPercent])).toEqual([
    ['monthly', 25],
    ['rpm', 40],
    ['tpm', 5],
  ])
})

test('percent helpers', () => {
  expect(clampUsedPercent(150)).toBe(100)
  expect(clampUsedPercent(-1)).toBe(0)
  expect(usedPercentFromRatio(25, 100)).toBe(25)
  expect(usedPercentFromRatio(1, 0)).toBeNull()
  expect(severityFromUsedPercent(90)).toBe('warning')
  expect(severityFromUsedPercent(99)).toBe('critical')
  expect(unixSecondsToIso(1_700_000_000)).toMatch(/^\d{4}-/)
  expect(unixSecondsToIso('2026-07-09T09:00:00.000Z')).toBe('2026-07-09T09:00:00.000Z')
})
