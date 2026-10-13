import { expect, test } from 'bun:test'
import type { ContextUsage } from '@demicodes/protocol'
import { compactionRefusal, contextCardRows, contextFigure, contextPercent, contextRatio, untilCompaction } from '../context-usage'

const WINDOW = 100_000

function usage(tokens: number) {
  return { tokens, window: WINDOW, compactFrom: WINDOW / 2, rows: [] }
}

test('Compact is refused below half the window, with the share it is at, as the backend refuses it', () => {
  expect(compactionRefusal(usage(23_400))).toBe('Compaction is available from 50% context usage (now 23%)')
  // Just under half reads 49%, never a refusal that says 50%.
  expect(compactionRefusal(usage(49_999))).toBe('Compaction is available from 50% context usage (now 49%)')
  expect(compactionRefusal(usage(50_000))).toBeNull()
})

test('a model without a window, or a usage not reported yet, leaves Compact to the backend', () => {
  expect(compactionRefusal({ tokens: 12_000, window: null, compactFrom: null, rows: [] })).toBeNull()
  expect(compactionRefusal(null)).toBeNull()
  expect(contextPercent({ tokens: 12_000, window: null, compactFrom: null, rows: [] })).toBeNull()
})

test('the card lists what the request carries largest first, then the compaction buffer, then the free space', () => {
  const card: ContextUsage = {
    tokens: 20_000,
    window: WINDOW,
    compactFrom: WINDOW / 2,
    // In the frame's order, which is not the card's.
    rows: [
      { kind: 'messages', tokens: 14_000, estimated: true },
      { kind: 'systemPrompt', tokens: 3_600, estimated: true },
      { kind: 'commands', tokens: 900, estimated: true },
      { kind: 'instructions', tokens: 1_200, estimated: true },
      { kind: 'otherContext', tokens: 300, estimated: true },
      { kind: 'compactionBuffer', tokens: 30_000, estimated: false },
      { kind: 'freeSpace', tokens: 50_000, estimated: false },
    ],
  }
  expect(contextCardRows(card).map((row) => row.label)).toEqual([
    'Messages',
    'System prompt',
    'Instructions',
    'Commands',
    'Other context',
    'Compaction buffer',
    'Free space',
  ])
  // A conversation whose messages are still smaller than its system prompt.
  const early = { ...card, rows: card.rows.map((row) => (row.kind === 'messages' ? { ...row, tokens: 200 } : row)) }
  expect(contextCardRows(early).map((row) => row.kind)).toEqual([
    'systemPrompt',
    'instructions',
    'commands',
    'otherContext',
    'messages',
    'compactionBuffer',
    'freeSpace',
  ])
})

test('the room before compaction is the threshold less the estimate, never below zero, and unknown without a window', () => {
  const at = (tokens: number): ContextUsage => ({
    tokens,
    window: 1_000_000,
    compactFrom: 500_000,
    rows: [{ kind: 'compactionBuffer', tokens: 33_000, estimated: false }],
  })
  // The threshold is the window less the buffer: 967K.
  expect(untilCompaction(at(291_000))).toBe(676_000)
  expect(untilCompaction(at(980_000))).toBe(0)
  expect(untilCompaction({ tokens: 12_000, window: null, compactFrom: null, rows: [] })).toBeNull()
})

test('the meter shows the estimate in whole percent of the window, rounded down, full at most', () => {
  expect(contextPercent(usage(62_500))).toBe('62%')
  expect(contextPercent(usage(29_000))).toBe('29%')
  expect(contextPercent(usage(130_000))).toBe('100%')
  expect(contextRatio(usage(130_000))).toBe(1)
})

test('the ring\'s tip is the figure of the card header: filled against the window and its share, the filled alone without a window', () => {
  expect(contextFigure({ tokens: 139_000, window: 1_000_000, compactFrom: 500_000, rows: [] })).toBe('139K / 1M (13%)')
  expect(contextFigure({ tokens: 12_000, window: null, compactFrom: null, rows: [] })).toBe('12K')
})
