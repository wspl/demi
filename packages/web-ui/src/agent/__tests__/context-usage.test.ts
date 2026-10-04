import { expect, test } from 'bun:test'
import { compactionRefusal, contextPercent } from '../context-usage'

const WINDOW = 100_000

function usage(tokens: number) {
  return { tokens, window: WINDOW, compactFrom: WINDOW / 2 }
}

test('Compact is refused below half the window, with the share it is at, as the backend refuses it', () => {
  expect(compactionRefusal(usage(23_400))).toBe('Compaction is available from 50% context usage (now 23%)')
  // Just under half reads 49%, never a refusal that says 50%.
  expect(compactionRefusal(usage(49_999))).toBe('Compaction is available from 50% context usage (now 49%)')
  expect(compactionRefusal(usage(50_000))).toBeNull()
})

test('a model without a window, or a usage not reported yet, leaves Compact to the backend', () => {
  expect(compactionRefusal({ tokens: 12_000, window: null, compactFrom: null })).toBeNull()
  expect(compactionRefusal(null)).toBeNull()
  expect(contextPercent({ tokens: 12_000, window: null, compactFrom: null })).toBeNull()
})

test('the ring shows the estimate in whole percent of the window, full at most', () => {
  expect(contextPercent(usage(62_500))).toBe(62)
  expect(contextPercent(usage(130_000))).toBe(100)
})
