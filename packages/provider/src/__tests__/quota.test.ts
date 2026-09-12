import { deferred } from '@demicodes/utils'
import { expect, test } from 'bun:test'
import {
  clampUsedPercent,
  createProviderQuota,
  ensureQuota,
  ProviderQuotaUnsupportedError,
  ProviderQuotaInvalidatedError,
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
      if (used == null)
        return null
      return {
        windows: [{ id: 'rpm', usedPercent: Number(used), resetsAt: null }],
      }
    },
  })

  expect(quota.capability()).toMatchObject({
    mode: 'supported',
    canProbe: true,
    canObserve: true
  })
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
  expect(
    observed?.windows.find((window) => window.id === 'rpm')?.usedPercent
  ).toBe(42)
  expect(
    quota.latest()?.windows.find((window) => window.id === 'rpm')?.usedPercent
  ).toBe(42)

  const ensured = await ensureQuota(
    quota,
    { prefer: 'cache', maxStaleMs: 60_000 }
  )
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
  await expect(quota.probe())
    .rejects.toBeInstanceOf(ProviderQuotaUnsupportedError)
})

test(
  'observations merge partial windows without dropping probed plan and quota',
  async () => {
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
    expect(
      observed?.windows.map((window) => [window.id, window.usedPercent])
    ).toEqual(
      [
        ['monthly', 25],
        ['rpm', 40],
        ['tpm', 5],
      ]
    )
  }
)

test(
  'clearing quota after an account switch prevents an older probe overwriting the new account',
  async () => {
    const old = deferred<void>()
    let account = 'A'
    const quota = createProviderQuota({
      providerId: 'demo', canProbe: true,
      probe: async () => {
        const requestedAccount = account
        if (requestedAccount === 'A')
          await old.promise
        return { accountLabel: requestedAccount, windows: [] }
      },
    })
    const probingA = quota.probe().catch((error: unknown) => error)
    account = 'B'
    quota.clearLatest!()
    await quota.probe()
    expect(quota.latest()?.accountLabel).toBe('B')
    old.resolve()
    expect(await probingA).toBeInstanceOf(ProviderQuotaInvalidatedError)
    expect(quota.latest()?.accountLabel).toBe('B')
  }
)

test(
  'captured observers ignore old account responses after the quota is cleared',
  () => {
    const quota = createProviderQuota({
      providerId: 'demo', canProbe: false,
      probe: async () => ({ windows: [] }),
      observe: ({ body }) => ({ accountLabel: String(body), windows: [] }),
    })
    const observeA = quota.captureObserver()
    expect(observeA({ body: 'A' })?.accountLabel).toBe('A')
    quota.clearLatest!()
    expect(quota.latest()).toBeNull()
    expect(observeA({ body: 'A' })).toBeNull()
    expect(quota.latest()).toBeNull()
    quota.captureObserver()({ body: 'B' })
    observeA({ body: 'A' })
    expect(quota.latest()?.accountLabel).toBe('B')
  }
)

test('percent helpers', () => {
  expect(clampUsedPercent(150)).toBe(100)
  expect(clampUsedPercent(-1)).toBe(0)
  expect(usedPercentFromRatio(25, 100)).toBe(25)
  expect(usedPercentFromRatio(1, 0)).toBeNull()
  expect(severityFromUsedPercent(90)).toBe('warning')
  expect(severityFromUsedPercent(99)).toBe('critical')
  expect(unixSecondsToIso(1_700_000_000)).toMatch(/^\d{4}-/)
  expect(unixSecondsToIso(0)).toBe('1970-01-01T00:00:00.000Z')
  expect(unixSecondsToIso(null)).toBeNull()
})

test('quota timestamp contracts use explicit seconds and preserve unknown values', async () => {
  const { quotaResetSchema, quotaEpochSecondsSchema } = await import('../quota-schemas')
  expect(quotaResetSchema.parse(1_700_000_000)).toBe('2023-11-14T22:13:20.000Z')
  expect(quotaResetSchema.parse('1700000000')).toBe('2023-11-14T22:13:20.000Z')
  expect(quotaResetSchema.parse('2026-07-09T09:00:00.000Z'))
    .toBe('2026-07-09T09:00:00.000Z')
  expect(unixSecondsToIso(1_700_000_000_000 / 1000)).toBe('2023-11-14T22:13:20.000Z')
  expect(quotaEpochSecondsSchema.safeParse(1e30).success).toBe(false)
  for (const value of ['', 'yesterday', '2026-02-30T00:00:00Z', [], {}, -1, Infinity]) {
    expect(quotaResetSchema.safeParse(value).success).toBe(false)
  }
})

test('number headers distinguish absent values from malformed decimal input', async () => {
  const { numberHeader } = await import('../http')
  expect(numberHeader(new Headers(), 'x-number')).toBeNull()
  expect(numberHeader(new Headers({ 'x-number': '0' }), 'x-number')).toBe(0)
  expect(numberHeader(new Headers({ 'x-number': '35.5' }), 'x-number')).toBe(35.5)
  for (const value of ['', ' ', '0x10', 'Infinity', 'NaN', 'private-value']) {
    try {
      numberHeader(new Headers({ 'x-number': value }), 'x-number')
      throw new Error('expected rejection')
    } catch (error) {
      expect(error).toMatchObject({ code: 'invalid_provider_response' })
      expect(String(error)).not.toContain('private-value')
    }
  }
})


test('quota math rejects corrupt internal measurements instead of treating them as unknown', () => {
  expect(() => clampUsedPercent(NaN)).toThrow()
  expect(() => usedPercentFromRatio(-1, 10)).toThrow()
  expect(() => usedPercentFromRatio(Infinity, 10)).toThrow()
  expect(usedPercentFromRatio(1e308, 1e-308)).toBe(100)
})
