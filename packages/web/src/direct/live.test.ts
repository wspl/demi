import { expect, test } from 'bun:test'
import { DeviceLive } from './live'
import type { DirectPeer } from './peer'

// The device's peer for live views (`direct-channel.md` § Making the
// channel): it closes a minute after its last view ends, counting the views
// of each peer apart, since a view of a peer that went still ends later.

function peer(): DirectPeer {
  const ended = Promise.withResolvers<void>()
  return {
    open: () => Promise.reject(new Error('not opened in this test')),
    attempt: {
      startedAt: '2026-10-19T09:00:00.000Z',
      durationMs: 20,
      outcome: 'connected',
      stage: null,
      browser: { local: [], public: [] },
      device: { local: ['127.0.0.1'], public: [] },
      pairs: { tried: 1, answered: 1 },
      inUse: null,
      permission: null,
    },
    probe: () => {},
    onProbe: () => () => {},
    closed: ended.promise,
    close: () => ended.resolve(),
  }
}

/**
 * A view of the old peer is still open when the new peer's only view
 * ends: the new peer goes idle all the same. Planted defect this catches:
 * one count of views across peers, which the old peer's view keeps above
 * zero, so the new peer never closes.
 */
test('the live peer closes a minute after its own last view, whatever views of an older peer remain', async () => {
  const idle: (() => void)[] = []
  const closed: string[] = []
  const peers = [peer(), peer()]
  let made = 0
  const live = new DeviceLive({
    connect: async () => peers[made++]!,
    closed: () => closed.push('live'),
    after: (_ms, run) => {
      idle.push(run)
      return () => {}
    },
  })
  const old = live.use()
  live.failed(await old.peer)
  await Promise.resolve()
  const current = live.use()
  expect(await current.peer).toBe(peers[1]!)
  current.release()
  expect(idle).toHaveLength(1)
  idle[0]!()
  expect(closed).toEqual(['live', 'live'])
  old.release()
  expect(idle).toHaveLength(1)
})
