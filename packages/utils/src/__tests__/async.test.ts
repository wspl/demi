import { describe, expect, it } from 'bun:test'
import { waitFor } from '../async'

describe('waitFor', () => {
  it('resolves once the predicate becomes true', async () => {
    let ticks = 0
    await waitFor(() => ++ticks >= 3, undefined, { intervalMs: 1 })
    expect(ticks).toBeGreaterThanOrEqual(3)
  })

  it('resolves immediately when already true', async () => {
    await waitFor(() => true)
  })

  it('rejects with the described context on timeout', async () => {
    await expect(
      waitFor(() => false, () => 'never ready', { timeoutMs: 5, intervalMs: 1 }),
    ).rejects.toThrow('Timed out waiting for condition: never ready')
  })
})
